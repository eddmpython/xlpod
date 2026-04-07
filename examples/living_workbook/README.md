# living_workbook — xlpod Lite panel demo (Phase 12)

This is the runnable demo of the xlpod *living workbook* idea. After
xlpod is installed (`pip install xlpod` for the desktop client; the
launcher binary is a separate download from GitHub Releases), an
xlwings Lite user pastes [`lite_panel.py`](lite_panel.py) into the
Lite editor and gets:

- Auto-discovery of the running launcher on
  `https://127.0.0.1:7421` (Phase 0 measurement confirmed Lite's CSP
  allows this).
- A handshake that requests `ai:provider:call`, `bundle:read`,
  `bundle:write`, `excel:com`, and `run:python`. The user approves
  the scope set through the launcher's tray dialog (Phase 4
  consent gate). The token's `fs_roots` is the directory of the
  current workbook so the bundle calls can find it.
- A chat panel that opens an AI session and sends messages through
  `/ai/chat`. The model has the launcher's tool registry (`fs_read`,
  `excel_workbooks`, `excel_range_read`, `run_python`) and can
  inspect / modify the workbook through the same 5-check stack a
  human caller would traverse.
- A **Save** action that captures the AI conversation history into
  the `.xlsx` itself via `/bundle/write`. A later opener can pick
  the conversation up where the original author left off.

## Run it

1. Start the launcher (`cargo run -p xlpod-server` from the repo
   for dev, or the signed `xlpod.exe` from GitHub Releases).
2. Set an Anthropic API key once via the launcher tray, or pass
   `ANTHROPIC_API_KEY=...` to the launcher process. The key is
   stored in the OS keychain — never echoed.
3. Open Excel + xlwings Lite. Paste `lite_panel.py` into the
   Lite editor.
4. Run `await main()` from the Lite Python tab. The panel
   handshake + session opens and the demo prints `[lite] ...`
   events to the Lite console.
5. From the same Lite session, drive the panel:

   ```python
   await panel.send("List the open workbooks and tell me their names.")
   await panel.save()  # writes the AI history into the workbook
   ```

6. Close the workbook. Re-open it. The panel detects the bundle
   via `/bundle/read` and emits `history_restored` — the next
   `panel.send(...)` call gets the previous conversation as
   context automatically.

## What is *not* in this demo

- A real GUI. The Lite panel host renders chat history from the
  events the script emits via `on_event(kind, payload)`. Phase 12
  is the *script*; the host UI is whatever the user writes around
  it. (`lite_panel.py` is intentionally framework-free.)
- Real Pyodide state snapshots. Phase 10's `BundlePayload`
  reserves a `pyodide_snapshot` field but Phase 12 stores AI
  history only — restoring a Pyodide image would require pinning
  the Pyodide version and pulling in dill, which is held over to
  Phase 13+.
- Streaming chat responses. The Phase 9 plan included SSE
  streaming but the launcher's `/ai/chat` is currently
  request/response only; the panel calls block on the full
  assistant turn.

## Headless smoke test

`client/tests/test_lite_panel.py` imports `lite_panel.py` via
`importlib.util` and exercises `LivingWorkbookPanel` against a
fake transport. It runs on every Python in the CI matrix
(3.10–3.13) so a refactor of the panel cannot silently break the
contract this README describes.
