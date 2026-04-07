# examples/

End-to-end demos that exercise the launcher + client across the
protocol boundary defined in
[`../proto/xlpod.openapi.yaml`](../proto/xlpod.openapi.yaml).

## desktop_hello.py — CPython sync `xlpod.Client`

```bash
# 1. Start the launcher (in a separate terminal)
cargo run -p xlpod-server     # or `cargo run -p xlpod-launcher` for the tray

# 2. Run the demo
uv run python examples/desktop_hello.py
```

What it proves:
- the launcher is reachable on `https://127.0.0.1:7421`
- TLS verification passes against the mkcert local CA (auto-discovered)
- `/health`, `/auth/handshake`, and `/launcher/version` all return the
  shapes declared in the OpenAPI spec
- the audit log under `%LOCALAPPDATA%/xlpod/audit.log` records the
  full call sequence with the token id but never the token itself

## lite_hello.py — Pyodide / xlwings Lite

This file is intentionally docs-only; the runnable snippet lives at the
top of the docstring. Open xlwings Lite in Excel, paste the snippet
into the Python tab, and the same three round trips happen inside the
browser sandbox. The transport autodetects `sys.platform == "emscripten"`
and switches to `pyodide.http.pyfetch`; no other code changes.

The Phase 0 measurement (see [`../docs/phase0-report.md`](../docs/phase0-report.md))
already proved that the underlying `fetch` from
`https://addin.xlwings.org` to `https://127.0.0.1:7421` is allowed by
the Lite CSP and trusted by WebView2.
