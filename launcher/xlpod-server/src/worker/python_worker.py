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
import sys
import traceback


def _send(payload):
    line = json.dumps(payload)
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


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
        elif method == "shutdown":
            _send({"id": rid, "ok": True})
            return
        else:
            _send({"id": rid, "ok": False, "error": "unknown method: {}".format(method)})


if __name__ == "__main__":
    _main()
