# xlpod Threat Model (STRIDE, draft v0)

Scope: the xlpod launcher (`xlpod.exe`) and its loopback HTTPS API,
including the Pyodide client that calls it from xlwings Lite.

## Assets

| Asset | Where it lives | Damage if compromised |
|---|---|---|
| Session bearer token | launcher RAM, client RAM | Full RCE on user PC within scope |
| Local CA private key | `%LOCALAPPDATA%\xlpod\ca\` | Attacker can MITM any localhost service |
| Audit log | `%LOCALAPPDATA%\xlpod\audit.log` | Forensic trail lost |
| User Excel files | filesystem | Data loss / exfiltration |
| Python worker stdin/stdout pipe | OS pipe | Code execution as the user |
| (Future) AI provider API key | Windows Credential Manager | Account abuse, billing fraud |

## STRIDE

### Spoofing
- **T1.** Hostile website tries to call the launcher from
  `https://evil.example`. **Mitigation:** Origin allow-list (§3),
  bearer token (§4), `Host` header check (§7).
- **T2.** DNS rebinding pins `attacker.com` to `127.0.0.1`.
  **Mitigation:** `Host` header must be the literal loopback string.
- **T3.** Hostile process on the same user account talks to the API.
  **Accepted residual risk** — at the same user privilege, the OS
  trust boundary is the user, not the process. Documented.

### Tampering
- **T4.** Attacker modifies `audit.log` to hide actions.
  **Mitigation:** append-only file, viewable from tray, future:
  hash-chain entries.
- **T5.** Update channel poisoned. **Mitigation:** Tauri updater +
  Ed25519 signed `latest.json`, GitHub Releases trusted publishing.

### Repudiation
- **T6.** "I didn't run that macro." **Mitigation:** Audit log records
  actor (user vs `ai:<provider>:<model>`), scope, path, status, time.

### Information disclosure
- **T7.** Token leaked via crash dump or log line.
  **Mitigation:** logger has a redaction middleware; tokens never
  serialized; strict log review in CI.
- **T8.** CA private key copied off the machine.
  **Mitigation:** restrictive ACL, current-user store only, uninstall
  removes it.

### Denial of service
- **T9.** Token holder floods the API. **Mitigation:** rate limit
  100 req/s/token (§9).
- **T10.** Worker stuck in infinite loop. **Mitigation:** wall-clock
  30s soft kill → hard kill, worker restart on >800MB.

### Elevation of privilege
- **T11.** `fs:read` token escalates to `fs:write`. **Mitigation:**
  scopes are immutable per token; new scope = new consent + new token.
- **T12.** AI prompt injection coerces an LLM into calling
  `excel:range/write` with destructive content. **Mitigations:**
  (a) human-in-the-loop default (§6), (b) `X-XLPod-Plan-Only` dry-run,
  (c) AI calls require the same tray consent as manual calls,
  (d) audit log distinguishes AI actor.

## Out of scope (v0)
- Other users on the same machine (multi-user threat).
- Physical attacker with disk access.
- Compromised Windows kernel / supply-chain compromise of `cryptography`,
  `rustls`, `tokio`. (We pin and verify, but cannot prove.)

## Review cadence
Update this document on every protocol change and on every new endpoint.
A PR adding an endpoint without a corresponding STRIDE entry is rejected.
