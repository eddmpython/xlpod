# xlpod Security Model

> If you believe you have found a security issue, **do not file a public
> issue**. Email the maintainers (address TBD before first release).

xlpod ships a local HTTPS server on the user's machine. A misdesigned
local server is a remote-code-execution channel for any visited website
(see Zoom 2019). Therefore *every* security control listed below is
mandatory and non-negotiable. There is no option to relax any of them;
options that exist get turned on.

## The 10 principles (defense in depth)

1. **Loopback only.** Bind `127.0.0.1` and `::1`. `0.0.0.0` is forbidden
   at compile time.
2. **TLS required.** Only `/health` may be served over plain HTTP, and
   only as a liveness probe.
3. **Origin allow-list.** Every request's `Origin` header is checked
   against a compiled-in list. Anything else: 403. The list is currently
   exactly one entry: **`https://addin.xlwings.org`** (xlwings Lite,
   confirmed by Phase 0 measurement, 2026-04-07). Wildcards are not
   honoured. The authoritative copy lives in
   [`../proto/xlpod.openapi.yaml`](../proto/xlpod.openapi.yaml) under
   `info.x-xlpod-allowed-origins`.
4. **Bearer token.** A 256-bit random token is issued at every launcher
   start. Required on every request except `/health`.
5. **Scoped permissions.** Tokens carry the minimum scopes:
   `fs:read`, `fs:write`, `run:python`, `excel:com`,
   and (reserved for future) `ai:provider:call`, `ai:codegen:write`,
   `ai:exec:python`. Filesystem scopes are *additionally* bound to a
   closed set of canonicalized `fs_roots` chosen at handshake time;
   `/fs/read` rejects any path that does not lie under one of them.
   A future tray consent dialog (Phase 4) will prompt the user before
   issuing a token that carries `fs_roots`.
6. **User consent.** Sensitive operations require an explicit, fresh
   tray confirmation. Consent grants narrow scopes to one token.
   Implementation lives in `xlpod_server::consent::ConsentBackend`:
   the production tray binary uses `MessageBoxConsent` (Win32
   `MessageBoxW`, `MB_TOPMOST | MB_SYSTEMMODAL`) which shows the
   requesting origin, the scopes, and the canonicalized fs roots
   before any token is minted. The standalone `xlpod-server` dev
   binary defaults to `AutoApproveConsent` for ergonomic smoke tests
   and integration runs; that backend is *never wired into the tray
   binary*, and a denial returns `consent_denied` with no token in
   the body.
7. **DNS-rebinding defense.** The `Host` header must equal
   `127.0.0.1:<port>` or `[::1]:<port>`.
8. **Audit log.** Every request is appended to a JSONL audit log
   (`audit.log`), viewable from the tray. The log records actor, scope,
   path, status, latency.
9. **Rate limit.** 100 req/s per token, hard fail above.
10. **CORS preflight.** `OPTIONS` is checked the same way as the actual
    method.

## Why we accept the risk Tauri's docs warn about

Tauri's official guidance is to prefer custom protocols over a localhost
plugin because localhost servers expose attack surface. We agree with
the warning. We use a localhost server anyway because our requirement
is **fetches from a third-party origin** (the xlwings Lite Office Add-in
iframe), not from our own webview. Custom protocols cannot satisfy
that. Every Tauri-listed risk is mitigated by the 10 principles above.

## Certificate handling

- The launcher generates a local CA on first run, stores the private
  key under `%LOCALAPPDATA%\xlpod\ca\` with restrictive ACL.
- The CA is registered in the **current user**'s root store only —
  never the machine store.
- Server certificates are issued for `127.0.0.1` and `::1` only.
- Uninstall removes both the CA file and the registry entry.

## Future: AI provider integration

When AI providers are added (Phase 6+, see plan), they enter the
existing security model rather than bypassing it:

- AI calls go launcher → provider, never client → provider. Keys live
  in Windows Credential Manager and never appear in audit logs (the
  logger has a redaction middleware).
- Every tool the AI calls is one of the existing scoped endpoints. No
  new attack surface.
- Audit log records `actor: "ai:<provider>:<model>"` so AI-driven and
  user-driven actions are distinguishable forensically.
- Default mode is "human-in-the-loop": every mutation requires the
  same tray consent that a manual call would.
- Mutation endpoints accept `X-XLPod-Plan-Only: 1` for dry-run
  preview before any state change.

## What is *not* security

These are good practice but not part of the model:
- Code formatting, lint rules.
- Telemetry (off by default, opt-in only).
- Update timing.

## Reporting

Until a permanent address exists, report via private GitHub Security
Advisory on the repository.
