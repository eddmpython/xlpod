# xlpod Python worker.
#
# Loop:
#   - read one JSON line from stdin
#   - dispatch on `method`
#   - write one JSON line to stdout
#
# The launcher serializes calls under a Mutex, so the loop is purely
# request/response without any concurrency. Stdout is reserved for the
# JSON response only — user `print()` is captured via redirect_stdout
# so it never collides with the framing. Same for stderr.
#
# Convention: a snippet may set the `_result` name; if it does, its
# `repr()` is returned in the response. This is the only escape hatch
# for returning a Python value through HTTP without going through
# JSON-incompatible types.
#
# This file is loaded by the Rust launcher via `include_str!` and
# passed to `python -c`. Keep it self-contained, stdlib-only, no
# `__future__` imports beyond what python 3.10 already provides.

import contextlib
import io
import json
import os
import sys
import traceback

# Phase 11 — make the launcher client package importable for the
# bundle_read / bundle_write RPC methods. The launcher passes
# `XLPOD_CLIENT_PATH` pointing at the directory containing
# `xlpod/__init__.py`; in production this is either the wheel-
# installed location or the repo's `client/` directory. We tolerate
# the env var being missing (older launcher) and only fail if a
# bundle method is actually called.
_CLIENT_PATH = os.environ.get("XLPOD_CLIENT_PATH")
if _CLIENT_PATH and _CLIENT_PATH not in sys.path:
    sys.path.insert(0, _CLIENT_PATH)


def _send(payload):
    line = json.dumps(payload)
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


# ---------------------------------------------------------------------------
# Excel backends
#
# Two implementations of the same surface (`workbooks()`, `range_read(...)`),
# selected by the XLPOD_WORKER_BACKEND environment variable:
#
#   auto    (default) — try xlwings first, fall back to pywin32 on
#                       ImportError. This is the production path now that
#                       Phase 7 has reconciled the worker with design.md §5
#                       (the launcher's job is to make xlwings work, then
#                       use it).
#   xlwings           — force xlwings, fail with `excel_not_available`
#                       otherwise. Used by integration tests on a box
#                       that has xlwings installed.
#   pywin32           — force the raw pywin32 path. Kept as a transition
#                       safety net and used by the integration test that
#                       proves the fallback is still wired up.
#
# Both backends return identical response shapes so the route handlers
# stay backend-agnostic. Errors that originate inside Excel itself
# (workbook not found, bad range, etc.) come back as
# `error_code: excel_failed` with a human-readable message.
# ---------------------------------------------------------------------------


class _BackendUnavailable(Exception):
    """Raised when the selected backend cannot be loaded (no module, no
    Excel instance). The dispatch layer translates this into a
    structured error_code response."""

    def __init__(self, code, message):
        super().__init__(message)
        self.code = code
        self.message = message


class _XlwingsBackend:
    name = "xlwings"

    def __init__(self):
        try:
            import xlwings as xw  # type: ignore
        except Exception as e:
            raise _BackendUnavailable("excel_not_available", str(e))
        try:
            self._app = xw.apps.active
        except Exception as e:
            raise _BackendUnavailable("excel_not_running", str(e))
        if self._app is None:
            raise _BackendUnavailable(
                "excel_not_running", "xlwings reports no active Excel application"
            )

    def workbooks(self):
        out = []
        for book in self._app.books:
            full = book.fullname
            # xlwings .fullname is the absolute path including the
            # filename when saved, or just the bare name when unsaved.
            if os.path.sep in full or "/" in full:
                path = os.path.dirname(full)
            else:
                path = ""
            out.append({"name": book.name, "path": path, "full_name": full})
        return out

    def range_read(self, workbook, sheet, address):
        book = self._app.books[workbook] if workbook else self._app.books.active
        sh = book.sheets[sheet] if sheet else book.sheets.active
        rng = sh.range(address)
        val = rng.value
        # xlwings returns scalars for single cells, lists for 1-D
        # ranges, and lists-of-lists for 2-D ranges. Normalize to 2-D.
        if val is None:
            values = [[None]]
        elif isinstance(val, list):
            if val and isinstance(val[0], list):
                values = [list(row) for row in val]
            else:
                values = [list(val)]
        else:
            values = [[val]]
        return {"address": str(rng.address), "values": values}


class _Pywin32Backend:
    name = "pywin32"

    def __init__(self):
        try:
            import win32com.client  # type: ignore
        except Exception as e:
            raise _BackendUnavailable("excel_not_available", str(e))
        try:
            self._app = win32com.client.GetActiveObject("Excel.Application")
        except Exception as e:
            raise _BackendUnavailable("excel_not_running", str(e))

    def workbooks(self):
        out = []
        for wb in self._app.Workbooks:
            out.append(
                {
                    "name": wb.Name,
                    "path": wb.Path or "",
                    "full_name": wb.FullName,
                }
            )
        return out

    def range_read(self, workbook, sheet, address):
        wb = self._app.Workbooks(workbook) if workbook else self._app.ActiveWorkbook
        sh = wb.Worksheets(sheet) if sheet else wb.ActiveSheet
        rng = sh.Range(address)
        val = rng.Value
        # COM returns tuples-of-tuples for multi-cell ranges and a
        # scalar for single cells. Normalize so the wire shape is
        # always 2-D.
        if val is None:
            values = [[None]]
        elif isinstance(val, tuple):
            normalized = []
            for row in val:
                if isinstance(row, tuple):
                    normalized.append(list(row))
                else:
                    normalized.append([row])
            values = normalized
        else:
            values = [[val]]
        return {"address": str(rng.Address), "values": values}


def _select_backend():
    """Try the backend selected by XLPOD_WORKER_BACKEND, fall back per
    the rules at the top of this section, and return either a live
    backend instance or a structured error_code dict."""
    pref = (os.environ.get("XLPOD_WORKER_BACKEND") or "auto").strip().lower()
    if pref == "xlwings":
        order = [_XlwingsBackend]
    elif pref == "pywin32":
        order = [_Pywin32Backend]
    else:
        order = [_XlwingsBackend, _Pywin32Backend]
    last_error = None
    for cls in order:
        try:
            return cls()
        except _BackendUnavailable as e:
            last_error = e
            continue
    code = last_error.code if last_error else "excel_not_available"
    message = last_error.message if last_error else "no excel backend available"
    return {"ok": False, "error_code": code, "message": message}


def _excel_workbooks():
    backend = _select_backend()
    if isinstance(backend, dict):
        return backend
    try:
        return {"ok": True, "workbooks": backend.workbooks()}
    except Exception as e:
        return {"ok": False, "error_code": "excel_failed", "message": str(e)}


def _excel_range_read(workbook, sheet, address):
    backend = _select_backend()
    if isinstance(backend, dict):
        return backend
    try:
        result = backend.range_read(workbook, sheet, address)
        return {"ok": True, **result}
    except Exception as e:
        return {"ok": False, "error_code": "excel_failed", "message": str(e)}


def _bundle_read(path):
    try:
        from xlpod.bundle import (  # type: ignore
            BundleNotFound,
            BundleReader,
            BundleSchemaMismatch,
            BundleTooLarge,
            BundleCorrupt,
        )
    except Exception as e:
        return {"ok": False, "error_code": "bundle_corrupt", "message": "client package missing: {}".format(e)}
    try:
        payload = BundleReader(path).read()
    except FileNotFoundError as e:
        return {"ok": False, "error_code": "path_not_found", "message": str(e)}
    except BundleNotFound as e:
        return {"ok": False, "error_code": "bundle_not_found", "message": str(e)}
    except BundleSchemaMismatch as e:
        return {"ok": False, "error_code": "bundle_schema_mismatch", "message": str(e)}
    except BundleTooLarge as e:
        return {"ok": False, "error_code": "bundle_too_large", "message": str(e)}
    except BundleCorrupt as e:
        return {"ok": False, "error_code": "bundle_corrupt", "message": str(e)}
    except Exception as e:
        return {"ok": False, "error_code": "bundle_corrupt", "message": str(e)}
    return {"ok": True, "payload": payload.to_dict()}


def _bundle_write(path, payload_dict):
    try:
        from xlpod.bundle import (  # type: ignore
            BundlePayload,
            BundleWriter,
            BundleTooLarge,
            BundleCorrupt,
        )
    except Exception as e:
        return {"ok": False, "error_code": "bundle_corrupt", "message": "client package missing: {}".format(e)}
    try:
        # Build a BundlePayload from the inbound dict, accepting both
        # the canonical to_dict() shape and a direct field mapping.
        if isinstance(payload_dict, dict) and payload_dict.get("schema"):
            payload = BundlePayload.from_dict(payload_dict)
        else:
            md = (payload_dict or {}).get("metadata") or {}
            ai = (payload_dict or {}).get("ai_history") or {}
            payload = BundlePayload(
                created_ms=int(md.get("created_ms", 0) or 0),
                ai_sessions=list(ai.get("sessions", [])),
                python_modules=list((payload_dict or {}).get("python_modules", [])),
            )
    except Exception as e:
        return {"ok": False, "error_code": "bundle_corrupt", "message": str(e)}
    try:
        BundleWriter(path).write(payload)
    except FileNotFoundError as e:
        return {"ok": False, "error_code": "path_not_found", "message": str(e)}
    except BundleTooLarge as e:
        return {"ok": False, "error_code": "bundle_too_large", "message": str(e)}
    except BundleCorrupt as e:
        return {"ok": False, "error_code": "bundle_corrupt", "message": str(e)}
    except Exception as e:
        return {"ok": False, "error_code": "bundle_corrupt", "message": str(e)}
    return {"ok": True}


def _exec(code):
    out = io.StringIO()
    err = io.StringIO()
    result = None
    error = None
    namespace = {"__name__": "__xlpod_worker__"}
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            exec(code, namespace)
            result = namespace.get("_result")
    except SystemExit as e:
        # SystemExit would otherwise terminate the worker; surface it
        # as a normal Python-level error instead.
        error = "SystemExit: {}".format(e.code)
    except BaseException as e:
        error = "".join(traceback.format_exception(type(e), e, e.__traceback__))
    return {
        "ok": error is None,
        "stdout": out.getvalue(),
        "stderr": err.getvalue(),
        "result": repr(result) if result is not None else None,
        "error": error,
    }


def _main():
    # Line-buffer stdout so the launcher sees responses immediately.
    try:
        sys.stdout.reconfigure(line_buffering=True)
    except Exception:
        pass

    while True:
        line = sys.stdin.readline()
        if not line:
            return
        try:
            msg = json.loads(line)
        except Exception as e:
            _send({"id": None, "ok": False, "error": "bad json: {}".format(e)})
            continue
        rid = msg.get("id")
        method = msg.get("method")
        params = msg.get("params") or {}
        if method == "ping":
            _send({"id": rid, "ok": True})
        elif method == "exec":
            payload = _exec(params.get("code", ""))
            payload["id"] = rid
            _send(payload)
        elif method == "excel_workbooks":
            payload = _excel_workbooks()
            payload["id"] = rid
            _send(payload)
        elif method == "excel_range_read":
            payload = _excel_range_read(
                params.get("workbook", ""),
                params.get("sheet", ""),
                params.get("range", ""),
            )
            payload["id"] = rid
            _send(payload)
        elif method == "bundle_read":
            payload = _bundle_read(params.get("path", ""))
            payload["id"] = rid
            _send(payload)
        elif method == "bundle_write":
            payload = _bundle_write(params.get("path", ""), params.get("payload", {}))
            payload["id"] = rid
            _send(payload)
        elif method == "shutdown":
            _send({"id": rid, "ok": True})
            return
        else:
            _send({"id": rid, "ok": False, "error": "unknown method: {}".format(method)})


if __name__ == "__main__":
    _main()
