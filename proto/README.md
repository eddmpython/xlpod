# proto/ — Single Source of Truth

`xlpod.openapi.yaml` is the **only** authoritative description of the xlpod
loopback API. The Rust launcher (`launcher/`) and the pure-python client
(`client/xlpod`, future) both derive from and validate against this file.
**Neither side imports the other.** This spec is their only contact surface.

## Change procedure (mandatory order)

1. Open a PR that modifies `xlpod.openapi.yaml` first.
2. Once that PR is merged, open follow-up PRs that update the launcher
   and/or the client to match.
3. **Reverse order is forbidden.** A code PR that introduces an endpoint
   not present in the spec is rejected on review.

This rule exists because *the moment two files claim to be the API, both
become wrong*. SSOT is the only defense against drift.

## Validation

Lint locally before pushing:

```bash
npx --yes @redocly/cli@latest lint proto/xlpod.openapi.yaml
```

CI runs the same lint (added in Phase 1.4). Zero warnings is required.

## Versioning

The `X-XLPod-Proto` header carries the proto version negotiated between
client and launcher.

- **Minor changes** (additive: new optional fields, new routes, new scopes)
  do **not** bump the header — old clients keep working.
- **Major changes** (renamed fields, removed routes, semantic shifts)
  bump the header. The launcher refuses requests with a non-matching
  `X-XLPod-Proto` value.

The OpenAPI document's `info.version` tracks the spec file revision and
is independent from launcher and client semvers.

## No code generation

We deliberately do **not** use OpenAPI codegen for either side.

- No single generator targets both Rust + axum *and* pyodide-compatible
  pure Python well.
- Generated code tends to grow into a parallel source of truth that the
  team starts editing, defeating the SSOT premise.
- The API surface is small (Phase 1: 4 routes). Hand-written conformance
  is cheaper than generator maintenance.

If this calculus changes (e.g. the API grows past ~30 routes), revisit.

## What lives here

- `xlpod.openapi.yaml` — the spec
- `README.md` — this file

Nothing else. No examples, no helper scripts, no per-language stubs.
Helpers go in `scripts/` or the consuming module.

## See also

- [`../docs/SECURITY.md`](../docs/SECURITY.md) — the threat model that pins
  every constraint declared in the spec.
- [`../docs/threat-model.md`](../docs/threat-model.md) — STRIDE per asset.
- [`../docs/design.md`](../docs/design.md) — overall design (the API
  section is a human-readable summary; this file is authoritative).
- [`../docs/phase0-report.md`](../docs/phase0-report.md) — the measurement
  that justified `connect-src https: wss:` and the single allowed origin.
