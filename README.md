# xlpod

> Workbooks that carry their own Python runtime, AI conversation,
> and consent gate. Email a `.xlsx`, recipient opens it, everything
> just works.

xlpod is a tray-resident loopback HTTPS launcher + a pure-python
client + a [Lite panel demo](examples/living_workbook/) that turns
ordinary Excel workbooks into self-contained living documents. The
launcher relays AI provider calls (Anthropic in v0.7), runs a
sandboxed Python worker for tool calls, and reads/writes a custom
XML part inside the `.xlsx` so the AI history travels with the
file.

xlpod is **not affiliated with Zoomer Analytics, xlwings, or
Microsoft**.

## What's in the box

| Component | Where | What it does |
|---|---|---|
| `xlpod-server` | `launcher/xlpod-server` (Rust) | HTTPS server on `127.0.0.1:7421`, 12 routes, 5-check security stack, JSON-RPC Python worker, Excel COM via xlwings (with raw pywin32 fallback) |
| `xlpod-launcher` | `launcher/xlpod-launcher` (Rust) | Tray binary; hosts the server and the `MessageBoxConsent` dialog |
| `xlpod` (PyPI) | `client/xlpod` (pure Python) | `AsyncClient` + `Client` for CPython and Pyodide; reuses the same wire on both |
| `xlpod.bundle` | `client/xlpod/xlpod/bundle.py` | Reads/writes the `urn:xlpod:bundle:v1` custom XML part inside any `.xlsx` |
| Lite panel | `examples/living_workbook/lite_panel.py` | The demo that ties it all together inside xlwings Lite |

## The 60-second demo

```bash
# 1. Launcher (dev)
cd launcher && cargo run -p xlpod-server &

# 2. Client demo (CPython)
uv run python examples/desktop_hello.py
```

You should see a 6-line dump of `health → handshake → version →
fs.read → run.python → ai.session` round-trips. With
`ANTHROPIC_API_KEY=...` exported, the last step actually runs a
Claude completion through the launcher.

For the full living-workbook demo (Lite panel + AI history saved
into the `.xlsx`), see [`examples/living_workbook/README.md`](examples/living_workbook/README.md).

## Security in one paragraph

The launcher binds **loopback only** (`127.0.0.1` + `::1`,
compile-time constants), serves **TLS only**, validates **Origin**
against a single allow-list entry (`https://addin.xlwings.org`),
checks **Host** to defeat DNS rebinding, requires a 256-bit
**bearer token** issued at every launcher start, gates every
mutation behind a **tray consent dialog**, exposes AI tools to the
model only via the **same five-check stack** an external client
would traverse (no shortcut, no internal-bypass origin), keeps API
keys in **OS keychain** never echoed, records every call in a
**JSONL audit log** + a separate **cost ledger** with a daily cap
(default $5/day), and ships **57 STRIDE entries** in
[`docs/threat-model.md`](docs/threat-model.md).

The single-source-of-truth API spec lives at
[`proto/xlpod.openapi.yaml`](proto/xlpod.openapi.yaml). Both the
launcher and the client are derived from / validated against it;
they do not import each other.

## Status

| | |
|---|---|
| Build | windows-latest, redocly, python 3.10–3.13 |
| Tests | 39 Rust integration + 44 Python (mostly fake-transport unit) |
| Phases shipped | 0 → 13, current commit `0.0.0`/v0.7.x |
| Live demo | `cargo run -p xlpod-server && uv run python examples/desktop_hello.py` returns exit 0 with all 6 routes round-tripping |
| Live AI | working with `ANTHROPIC_API_KEY` set; `ai_provider_unconfigured` graceful path otherwise |

## License

Apache-2.0. See [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE).
Embedded dependencies (rustls, axum, hyper, reqwest, tokio,
xlwings as an optional worker backend, pywin32 fallback) keep
their own licenses.
