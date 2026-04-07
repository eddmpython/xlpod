# Phase 0 CSP probe

**This is the single go/no-go experiment for the entire xlpod project.**

The launcher concept depends on xlwings Lite (Pyodide, running inside an
Office Add-in iframe) being *allowed by its CSP* to call
`https://127.0.0.1:7421`. The `connect-src` directive is undocumented; we
must measure it.

## Run

```bash
uv sync --group phase0
uv run --group phase0 python scripts/phase0_csp_probe/probe.py
```

Local sanity check (cert warning is expected — we use a throwaway self-signed
cert and do **not** touch the system trust store):

```bash
curl --insecure https://127.0.0.1:7421/health
```

Then in Excel → xlwings Lite → DevTools (F12) Console, paste the snippet
that the script prints.

## Recording results

Write the outcome in [`../../docs/phase0-report.md`](../../docs/phase0-report.md):

| Outcome | Meaning | Next step |
|---|---|---|
| **GREEN** | fetch succeeds (or fails only with a known cert error after mkcert) | Begin Phase 1 |
| **YELLOW** | only TLS trust fails | Add mkcert step, re-test, then Phase 1 |
| **RED**   | CSP `connect-src` blocks `https://127.0.0.1` | **Project redesign** — Desktop-only or upstream CSP change |

## Hard rules

- This probe is **measurement only**. Do not extend it. Do not import it
  from launcher/ or client/. The real launcher is Rust + axum + rustls.
- Loopback only. Never bind 0.0.0.0.
- Throwaway cert. Never install into the system trust store.
- The directory will be deleted automatically by the OS temp cleanup; you
  can also remove it manually after the measurement.
