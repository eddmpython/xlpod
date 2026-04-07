# Phase 0 Report — xlwings Lite CSP `connect-src` measurement

**Status:** 🟢 **GREEN — project go.** Measured 2026-04-07.

## Result summary

- xlwings Lite iframe origin: **`https://addin.xlwings.org`** (confirmed via DevTools tab title `addin.xlwings.org/taskpane?...`)
- CSP `connect-src` **allows** `https://127.0.0.1:7421` — fetch reached the server (no `Refused to connect` violation message; only a TLS trust failure on the first attempt).
- After installing mkcert local CA into the Windows system trust store and re-issuing the probe cert for `127.0.0.1`/`::1`/`localhost`, WebView2 trusted the cert and `fetch('https://127.0.0.1:7421/health')` returned `STATUS 200 {"status":"ok","probe":"phase0","proto":"0"}`.
- Conclusion: design.md §3.4 (mkcert + Windows root store) is a viable path. Phase 1 (launcher MVP) is unblocked.

## Captured CSP (full Response Header from `taskpane?et=...` document)

```
default-src 'self';
frame-src 'none';
base-uri 'self';
form-action 'self';
object-src 'none';
upgrade-insecure-requests;
block-all-mixed-content;
frame-ancestors https://*.microsoft.com https://*.office.com
                https://*.officeapps.live.com https://*.sharepoint.com
                https://*.microsoft365.com https://*.cloud.microsoft
                https://teams.microsoft.com;
img-src 'self' data: blob:;
font-src 'self' data: https://res-1.cdn.office.net;
style-src 'self' 'unsafe-inline' https://res-1.cdn.office.net;
script-src 'self' 'wasm-unsafe-eval' blob:
           https://appsforoffice.microsoft.com
           https://cdn.jsdelivr.net
           https://plausible.io;
connect-src https: wss:;
worker-src 'self' blob:;
```

### Decisive findings

- **`connect-src https: wss:`** — any HTTPS/WSS endpoint is reachable. `https://127.0.0.1:<any-port>` and `wss://127.0.0.1:<any-port>` both unblocked. The launcher can use REST + WebSocket (`/ws`) without restriction.
- **`upgrade-insecure-requests` + `block-all-mixed-content`** — plain HTTP from Lite is *impossible*. Implication: design.md's "`/health` may be plain HTTP" exception is unreachable from Lite and should be **removed** from the launcher to shrink attack surface (one fewer code path = fewer bugs).
- **`frame-ancestors` is restricted to Microsoft domains** — a hostile site cannot iframe-embed Lite to phish a token. CSP gives us a free defense layer against one DNS-rebinding-adjacent vector.
- **`script-src` excludes `unsafe-eval`** (`wasm-unsafe-eval` only) — confirms Pyodide runs as Wasm, not via JS eval. Our pure-python client stays compatible.

### Action items derived from CSP
1. Drop the plain-HTTP `/health` exception from the launcher spec — TLS for everything.
2. Origin allow-list in launcher: **only** `https://addin.xlwings.org` (and any future xlwings self-hosted origin we explicitly opt in). Do **not** widen to `https://*.office.com` etc.; those are *frame-ancestors* of Lite, not the fetch origin.
3. Document this CSP snapshot in `proto/xlpod.openapi.yaml` v0 as the assumed client environment.

## Raw evidence

### Pre-mkcert attempt (self-signed cert)
```
GET https://127.0.0.1:7421/health net::ERR_CERT_AUTHORITY_INVALID
Uncaught (in promise) TypeError: Failed to fetch
```
→ TLS trust failed, **but no CSP violation** — proves connect-src is open.

### Post-mkcert attempt (trusted cert)
```
(async()=>{const r=await fetch('https://127.0.0.1:7421/health');console.log('STATUS',r.status,await r.text())})()
STATUS 200 {"status":"ok","probe":"phase0","proto":"0"}
```



This is the single go/no-go gate for the entire xlpod project. Until
this file shows GREEN or YELLOW, no `launcher/` or `client/` code is
written.

## How to fill this in

1. `uv sync --group phase0`
2. `uv run --group phase0 python scripts/phase0_csp_probe/probe.py`
3. Open xlwings Lite in Excel → F12 → paste the snippet the script
   prints → record everything below.

## Measurements

### M1. Lite iframe origin
```
location.origin = (paste here)
```

### M2. Response headers (from DevTools Network tab on the Lite document)
```
Content-Security-Policy: (paste full header)
Permissions-Policy: (paste)
Cross-Origin-Opener-Policy: (paste)
Cross-Origin-Embedder-Policy: (paste)
```

### M3. `connect-src` directive (extracted from M2)
```
connect-src (paste)
```

### M4. fetch result
```
(paste console output: status code + body, or full error)
```

### M5. CSP violation report (if any)
```
(paste any "Refused to connect to ... because it violates ..." messages)
```

## Verdict

- [ ] **GREEN** — fetch returns 200 (cert error after `--insecure` is OK).
      Begin Phase 1.
- [ ] **YELLOW** — only the TLS trust step fails. Add mkcert root-store
      install to the launcher install flow, re-test, then begin Phase 1.
- [ ] **RED** — `connect-src` blocks `https://127.0.0.1`.
      **Project redesign required.** Options:
      (a) Desktop-only scope, drop the Lite integration.
      (b) Upstream change request to xlwings to relax CSP for
          `https://127.0.0.1:*`.
      (c) Re-route via a same-origin proxy hosted on
          `addin.xlwings.org` — defeats the security model, **rejected**.

## Decision log
| Date | Author | Outcome | Notes |
|---|---|---|---|
| 2026-04-07 | maintainer | 🟢 GREEN | `connect-src https: wss:` 확인. mkcert 1.4.4 + Windows root store + WebView2 trust 동작. design.md 동기화 완료. |
