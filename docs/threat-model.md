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

## Phase 1.1–1.4 deltas

New surface introduced after the initial draft and the threats it brings:

### From Phase 1.1a (axum + rustls server)
- **T13.** Server bound to a non-loopback address by accident.
  **Mitigation:** `BIND_V4`/`BIND_V6` are compile-time `IpAddr` constants
  in [`launcher/xlpod-server/src/bind.rs`](../launcher/xlpod-server/src/bind.rs)
  guarded by a `const _: () = …` static assertion that panics the build
  if `BIND_V4` is not in `127.0.0.0/8`. Workspace lints additionally
  `deny(unsafe_code)` so a quick `0.0.0.0` patch cannot bypass via FFI.

### From Phase 1.2 (5-check stack + tokens + audit)
- **T14.** Bearer token leaked through a log line, panic message, or
  crash dump. **Mitigation:** the audit log records only `token_id`
  (first 8 hex chars of the token), never the secret; the `Handshake`
  schema tags `token` as `format: password`; tokens live exclusively in
  process memory inside `TokenStore` and are never persisted. **Open:**
  Windows minidumps still capture process memory — accepted residual
  risk for Phase 1; a future revision should `mlock` / `VirtualLock` the
  token store pages.
- **T15.** Audit log file tampered with by a process running as the
  same user. **Mitigation:** the file is opened with `append` mode so
  in-process writes always go at the tail; cross-process tampering is
  in-scope for the same-user threat we already accept (T3). A future
  revision should hash-chain entries so post-hoc edits are detectable.
- **T16.** Per-token rate limiter exhausted by anonymous flood on
  `/auth/handshake` (no token yet, no per-token bucket).
  **Mitigation:** Phase 1.2 enforces Origin + Host on `/auth/handshake`,
  which already restricts callers to xlwings Lite. A per-IP limiter is
  out of scope for a loopback-only server; documented.
- **T17.** Reserved AI scope smuggled in early via a handshake.
  **Mitigation:** `Scope::is_reserved()` rejects any handshake whose
  `requested_scopes` contains an `ai:*` value, returning `reserved_scope`
  with HTTP 400, *before* a token is issued.

### From Phase 1.3 (rcgen self-CA, install deferred)
- **T18.** Local CA private key (`rootCA-key.pem`) read by another
  process running as the same user. **Mitigation:** the file lives under
  `%LOCALAPPDATA%\xlpod\ca\` which inherits the per-user ACL. **Open:**
  we do not yet apply an explicit DACL that *denies* read to other
  identities; tracked for Phase 1.4.
- **T19.** Win32 `CertAddEncodedCertificateToStore` FFI parameter
  confusion (length/encoding mismatch). **Mitigation:** the only
  `unsafe` block in the workspace carries an inline `// SAFETY:` proof
  enumerating every pointer/length the call relies on, the workspace
  lint is `deny(unsafe_code)` so any *new* unsafe block requires an
  explicit `#[allow]`, and CI rejects clippy warnings to keep the
  exception list reviewable.

### From Phase 1.1b (tao + tray-icon launcher)
- **T20.** Tray "Quit" terminates the process while the server is
  mid-write to the audit log, truncating the trailing JSON line.
  **Mitigation:** the audit appender flushes after every entry, so the
  worst case is losing the in-flight request, not corrupting earlier
  history. Phase 1.4 will route Quit through a tokio
  `CancellationToken` so the server drains gracefully.
- **T21.** Worker thread that runs the server panics, but the tray
  thread keeps the process alive with no functioning HTTP surface.
  **Mitigation:** Phase 1.4 will install a `std::thread` panic hook
  that calls `process::exit(1)` so a dead server cannot present a
  green tray.

### From Phase 3 (`fs:read` scoped route)
- **T23.** Token with `fs:read` reads a file outside the user's intent.
  **Mitigation:** the token is bound at handshake time to a *closed
  set* of canonicalized `fs_roots`; every `/fs/read` call canonicalizes
  the requested path and rejects with `forbidden_path` if it does not
  start with one of the granted roots. The roots are canonicalized
  when the token is issued so a later symlink swap cannot widen them.
- **T24.** Path traversal via `..` segments. **Mitigation:** the
  handler does **not** parse for `..` strings (which is fragile);
  instead it calls `std::fs::canonicalize` first, which resolves the
  path against the real filesystem. The resulting absolute path is
  then compared against the canonicalized roots. A request like
  `/allowed/../other/secret` resolves to `/other/secret` and fails the
  `starts_with(allowed)` check. Covered by
  `fs_read_traversal_is_caught_by_canonicalize` in the integration
  tests.
- **T25.** Memory exhaustion via a multi-gigabyte file. **Mitigation:**
  `fs::metadata().len()` is checked against `FS_READ_MAX_BYTES`
  (10 MiB) *before* the read; oversized files return `path_too_large`
  without ever allocating a buffer. A streaming follow-up route will
  land alongside `/fs/list` for legitimate large reads.
- **T26.** Token with `fs:read` scope but no fs roots is silently
  useless and the caller does not learn until the first 403.
  **Mitigation:** `/auth/handshake` returns `bad_request` when
  `fs:read` is requested with an empty (or all-invalid) `fs_roots`
  list, so misconfiguration is caught at issue time.
- **T27.** Symlink swap after canonicalize but before read (TOCTOU).
  **Mitigation accepted:** Phase 3 follows symlinks deliberately and
  treats the user-approved roots as the trust boundary. A future
  "no-symlink" mode for stricter callers is tracked but not blocking.
- **T28.** A non-file (directory, FIFO, device, socket) is read and
  returns garbage / hangs. **Mitigation:** `metadata().is_file()` is
  required; everything else gets `not_a_file`.

### From Phase 1.4 (CI + commit-msg hook)
- **T22.** AI-tool attribution slips into a commit message. Not a
  security threat per se, but a policy violation that erodes trust.
  **Mitigation:** [`.githooks/commit-msg`](../.githooks/commit-msg) is
  the local enforcement point; the `no-ai-traces` job in
  `.github/workflows/ci.yml` is the server-side mirror. There is no
  `--no-verify` escape — a missed local install just fails CI.

## Out of scope (v0)
- Other users on the same machine (multi-user threat).
- Physical attacker with disk access.
- Compromised Windows kernel / supply-chain compromise of `cryptography`,
  `rustls`, `tokio`. (We pin and verify, but cannot prove.)

## Review cadence
Update this document on every protocol change and on every new endpoint.
A PR adding an endpoint without a corresponding STRIDE entry is rejected.
