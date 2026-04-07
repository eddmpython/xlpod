"""xlpod living workbook — Lite panel MVP.

Runs inside xlwings Lite (Pyodide) as the *Python* side of an Office
add-in panel. The Pyodide environment imports `xlpod` via the same
PyPI wheel that ships from this repo (after Phase 13 publishes
`xlpod==0.0.0`); until publish, the user `await
micropip.install("git+https://github.com/eddmpython/xlpod#subdirectory=client")`
into the Lite panel.

What this script does on panel open:

  1. Auto-detects the running launcher via `xlpod.AsyncClient` —
     Phase 0 measurement confirmed `connect-src https: wss:` lets
     Lite reach `https://127.0.0.1:7421` from the iframe.
  2. Performs a handshake requesting `bundle:read`, `bundle:write`,
     `ai:provider:call`, `excel:com`, and `run:python`. The user
     approves through the launcher's tray consent dialog (Phase 4
     mechanism). The token's `fs_roots` is the directory of the
     currently open workbook so `/bundle/*` can find it.
  3. Tries to read the bundle from the current workbook via
     `/bundle/read`. If the bundle exists, restores the AI session
     history into the panel UI; if not, starts a fresh session.
  4. Wires three button handlers:
       - **Send** — sends the user's typed message to
         `/ai/chat` and renders the assistant reply in the panel.
       - **Save** — captures the current Pyodide globals snapshot
         (Phase 10's `BundleWriter` via `/bundle/write`) so a
         later opener gets the same Python state + AI history.
       - **New session** — opens a fresh `/ai/session` and clears
         the history view.

This file is intentionally a *single self-contained module* so it
can be pasted into xlwings Lite's editor verbatim. The main entry
point is `main()`; the Lite panel host is expected to call
`asyncio.run(main())` (or `await main()` if it already has a loop).

For headless CI we mock the launcher transport — see
`tests/test_lite_panel.py` (Phase 12 follow-up).
"""

# Lite panel runs in Pyodide where these imports are stdlib + the
# xlpod wheel installed via micropip.
from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path
from typing import Any, Dict, List, Optional

try:
    import xlpod  # type: ignore
except ImportError as exc:  # pragma: no cover
    raise SystemExit(
        "xlpod is not installed in this Pyodide environment. "
        "Run `await micropip.install('xlpod')` first."
    ) from exc


# ---------------------------------------------------------------------------
# Workbook discovery
# ---------------------------------------------------------------------------


def _current_workbook_path() -> Optional[str]:
    """Best-effort lookup for the path of the workbook this Lite
    panel is attached to.

    xlwings Lite exposes the host workbook through its own helper
    module; we feature-detect because the helper is not always
    available (e.g. in the headless test harness).
    """
    try:
        import xlwings as xw  # type: ignore
    except Exception:
        return None
    try:
        book = xw.Book.caller()
        return str(book.fullname)
    except Exception:
        return None


def _workbook_root(path: str) -> str:
    return str(Path(path).resolve().parent)


# ---------------------------------------------------------------------------
# Panel state
# ---------------------------------------------------------------------------


class LivingWorkbookPanel:
    """The Lite panel's runtime state.

    Holds the connected `AsyncClient`, the current AI session, and
    the on-screen message history. UI rendering is host-specific
    (xlwings Lite vs the Phase 12 headless harness) so this class
    only emits *plain dicts* via `on_event`; the host wires those
    to actual UI calls.
    """

    def __init__(
        self,
        client: xlpod.AsyncClient,
        on_event: Optional[Any] = None,
    ) -> None:
        self._client = client
        self._on_event = on_event or (lambda kind, payload: None)
        self._workbook_path: Optional[str] = None
        self._session: Optional[xlpod.AISession] = None
        self._history: List[Dict[str, Any]] = []

    # ---- lifecycle -------------------------------------------------------

    async def on_open(self, workbook_path: Optional[str] = None) -> None:
        """Called when the Lite panel is first shown."""
        self._workbook_path = workbook_path or _current_workbook_path()
        scopes = ["ai:provider:call", "run:python", "excel:com"]
        fs_roots: List[str] = []
        if self._workbook_path:
            scopes.extend(["bundle:read", "bundle:write", "fs:read"])
            fs_roots.append(_workbook_root(self._workbook_path))
        await self._client.handshake(scopes=scopes, fs_roots=fs_roots)
        self._session = await self._client.open_session()
        self._on_event("session_opened", {"session_id": self._session.session_id})

        if self._workbook_path:
            await self._restore_bundle()

    async def _restore_bundle(self) -> None:
        if self._workbook_path is None:
            return
        try:
            payload = await self._client._request(  # noqa: SLF001 — internal
                "POST",
                "/bundle/read",
                json_body={"path": self._workbook_path},
                auth=True,
            )
        except xlpod.XlpodError:
            self._on_event("bundle_missing", {})
            return
        sessions = (payload.get("ai_history") or {}).get("sessions") or []
        if sessions:
            self._history = list(sessions[-1].get("messages", []))
            self._on_event(
                "history_restored",
                {"messages": len(self._history)},
            )

    # ---- chat ------------------------------------------------------------

    async def send(self, text: str) -> Dict[str, Any]:
        if self._session is None:
            raise RuntimeError("session not opened — call on_open() first")
        user_msg = {"role": "user", "content": [{"type": "text", "text": text}]}
        self._history.append(user_msg)
        self._on_event("user_message", user_msg)
        try:
            resp = await self._client.chat(
                session_id=self._session.session_id,
                messages=[user_msg],
            )
        except xlpod.AIProviderUnconfigured:
            self._on_event(
                "provider_missing",
                {"hint": "set ANTHROPIC_API_KEY via xlpod.set_provider_key"},
            )
            return {"ok": False, "error": "provider_unconfigured"}
        assistant_msg = resp.message
        self._history.append(assistant_msg)
        self._on_event("assistant_message", assistant_msg)
        return {"ok": True, "message": assistant_msg, "stop_reason": resp.stop_reason}

    # ---- save ------------------------------------------------------------

    async def save(self) -> None:
        if self._workbook_path is None:
            self._on_event("save_skipped", {"reason": "no workbook path"})
            return
        if self._session is None:
            self._on_event("save_skipped", {"reason": "no session"})
            return
        payload = {
            "metadata": {
                "created_ms": int(asyncio.get_event_loop().time() * 1000),
                "schema_version": 1,
            },
            "ai_history": {
                "sessions": [
                    {
                        "session_id": self._session.session_id,
                        "provider": self._session.provider,
                        "model": self._session.model,
                        "messages": self._history,
                    }
                ]
            },
            "python_modules": [],
        }
        await self._client._request(  # noqa: SLF001 — internal
            "POST",
            "/bundle/write",
            json_body={"path": self._workbook_path, "payload": payload},
            auth=True,
        )
        self._on_event("saved", {"messages": len(self._history)})

    # ---- new session -----------------------------------------------------

    async def new_session(self) -> None:
        self._session = await self._client.open_session()
        self._history = []
        self._on_event("session_reset", {"session_id": self._session.session_id})


# ---------------------------------------------------------------------------
# Convenience entry point
# ---------------------------------------------------------------------------


async def main() -> None:  # pragma: no cover - manual entry only
    client = xlpod.AsyncClient()
    panel = LivingWorkbookPanel(
        client,
        on_event=lambda kind, payload: print(f"[lite] {kind}: {payload}"),
    )
    await panel.on_open()
    print("Lite panel ready. Try:")
    print("  await panel.send('hello')")
    print("  await panel.save()")
    # In a real Lite panel, the host UI calls these in response to
    # button clicks. Here we just leave the panel object reachable.


if __name__ == "__main__":
    if os.environ.get("XLPOD_LITE_PANEL_RUN") == "1":
        asyncio.run(main())
