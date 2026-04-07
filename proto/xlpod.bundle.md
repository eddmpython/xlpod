# xlpod workbook bundle — `urn:xlpod:bundle:v1`

The xlpod *bundle* is a small JSON document the launcher writes into
an `.xlsx` file as a [Custom XML
Part](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/),
turning an ordinary workbook into a self-contained living document.
Receivers who open the workbook in Excel + xlwings Lite get the same
Pyodide environment + AI conversation history that the original
author left behind, with no separate install or setup.

This file is the SSOT for the bundle schema. Both
`client/xlpod/xlpod/bundle.py` (zip reader/writer) and the launcher's
worker call sites are derived from it; mismatches are fixed by
updating this file first and then the implementations.

## Where it lives

`.xlsx` is a ZIP container. Office Open XML defines the
`/customXml/itemN.xml` slot for arbitrary application data; we use a
single part with content type
`application/vnd.xlpod.bundle+json` and item path
`/customXml/xlpodBundle.json`. The OOXML `[Content_Types].xml` is
patched to register the part.

The xlpod namespace is the bare URI `urn:xlpod:bundle:v1`. The
xlwings Lite add-in already stores Python source under its own
namespace; we never collide because Lite uses a different file path
inside the zip.

## Wire shape

```json
{
  "schema": "urn:xlpod:bundle:v1",
  "metadata": {
    "created_ms": 1733600000000,
    "schema_version": 1,
    "launcher_min_version": "0.7.0",
    "workbook_fingerprint": "sha256:..."
  },
  "pyodide": {
    "encoding": "base64+zstd",
    "snapshot": "..."
  },
  "ai_history": {
    "sessions": [
      {
        "session_id": "uuid-v4",
        "provider": "anthropic",
        "model": "claude-opus-4-6",
        "opened_ms": 1733600000000,
        "closed_ms": 1733603600000,
        "messages": [
          {"role": "user", "ts_ms": 1733600100000, "content": [{"type": "text", "text": "..."}]},
          {"role": "assistant", "ts_ms": 1733600101000, "content": [
            {"type": "text", "text": "..."},
            {"type": "tool_use", "id": "tu_1", "name": "excel_workbooks", "input": {}}
          ]},
          {"role": "tool", "ts_ms": 1733600102000, "content": [
            {"type": "tool_result", "tool_use_id": "tu_1", "ok": true, "output": {...}, "approved_via": "auto"}
          ]}
        ]
      }
    ]
  },
  "python_modules": ["pandas", "numpy"]
}
```

### Field semantics

- `schema` — namespace URI; clients must accept any other value
  forward-compat by ignoring the bundle and treating the workbook
  as plain xlsx.
- `metadata.created_ms` — first time the bundle was written. Updated
  on every subsequent write only if a new launcher version writes
  it (so a Phase 10 launcher reading a Phase 11 bundle preserves the
  newer field).
- `metadata.schema_version` — integer; bumped when the *shape*
  changes incompatibly. Phase 10 ships `1`. Mismatch with the
  reader's expected version returns `BundleSchemaMismatch` and the
  launcher does not attempt to migrate.
- `metadata.launcher_min_version` — semver string the launcher
  refuses to load below.
- `metadata.workbook_fingerprint` — sha256 of the *cells only*
  (not the bundle), used to detect that a recipient's workbook
  diverged from the original after the bundle was written. The
  launcher warns before continuing on mismatch.
- `pyodide.encoding` — `"base64+zstd"` if zstd is available at
  build time, `"base64+zlib"` otherwise. Phase 10's bundle reader
  detects which by sniffing the first bytes of the decoded blob.
- `pyodide.snapshot` — Pyodide WASM linear memory snapshot per the
  Cloudflare format (Jan 2026), or a `dill` pickle of the worker
  globals if snapshots are unavailable. Phase 10 stores opaque
  bytes; the actual restore wiring lives in Phase 12 (Lite panel).
- `ai_history.sessions` — array of session transcripts in the same
  shape `/ai/session/{id}/history` returns from Phase 8. Both
  text and tool_use/tool_result blocks are stored, plus the
  `approved_via` audit hint. No bearer tokens, no API keys.
- `python_modules` — names of pip packages the snapshot relies on,
  for graceful fallback when a recipient cannot restore the snapshot
  but can still micropip-install the same set.

## Hard caps

- Total bundle bytes: **64 MiB**. Larger writes are rejected with
  `BundleTooLarge`. Practically the snapshot dominates and most
  workbooks land under 5 MiB.
- The bundle must round-trip through Excel's own save/load cycle
  without corruption. The reader/writer keeps Lite's own custom
  parts intact when modifying the file.

## Change procedure

1. PR updates this file first.
2. PR adds matching code under `client/xlpod/xlpod/bundle.py` and
   the worker dispatch (Phase 11).
3. STRIDE entries (`docs/threat-model.md`) for any new attack
   surface land in the same PR as the code.

## Why JSON, not XML

The OOXML format requires a custom XML part name, but the *body*
can be any well-formed XML. We chose JSON wrapped in a tiny XML
envelope so every consumer (Lite panel running Pyodide, Python
client running CPython, future MCP servers) speaks JSON natively
without an XML library. The wrapper is exactly:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<xlpodBundle xmlns="urn:xlpod:bundle:v1">
  <body><![CDATA[ {...JSON above...} ]]></body>
</xlpodBundle>
```
