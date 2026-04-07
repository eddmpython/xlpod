# xlpod

Pure-python client for the xlpod loopback launcher. Talks to the local
launcher over HTTPS following the protocol defined in
[`proto/xlpod.openapi.yaml`](https://github.com/eddmpython/xlpod/blob/main/proto/xlpod.openapi.yaml).

xlpod is an independent open-source project, **not affiliated with
Zoomer Analytics or Microsoft**. It is the client half of the xlpod
launcher; the launcher itself ships from
<https://github.com/eddmpython/xlpod/releases>.

## Install

```bash
pip install xlpod
```

In Pyodide / xlwings Lite:

```python
import micropip
await micropip.install("xlpod")
```

The CPython install pulls in `httpx`. Pyodide installs skip httpx and
use the built-in `pyodide.http` instead — same API, same client class.

## Usage

```python
import xlpod

# Sync (CPython)
with xlpod.Client() as c:
    print(c.health())
    c.handshake(scopes=["fs:read"])
    print(c.version())

# Async (Pyodide / xlwings Lite, or CPython if you prefer)
import asyncio

async def main():
    c = xlpod.AsyncClient()
    print(await c.health())
    await c.handshake(scopes=["fs:read"])
    print(await c.version())

asyncio.run(main())
```

## What it does (today)

Phase 2 covers the four routes the launcher exposes in Phase 1:

- `health()` — liveness probe (no auth)
- `handshake(scopes=[...])` — issue a bearer token, store it
- `version()` — bounded by the bearer token

Future scoped calls (`fs.read`, `excel.range`, `run.python`, …) land
alongside the launcher PRs that add them.

## Security model in one paragraph

The launcher binds **loopback only** (`127.0.0.1` + `::1`), serves
**TLS only**, validates **Origin** against a single allow-list entry
(`https://addin.xlwings.org`), checks **Host** to defeat DNS rebinding,
requires a **256-bit bearer token** issued at every launcher start,
and writes a **JSONL audit log**. Reserved AI scopes (`ai:*`) are
rejected at handshake until Phase 6. See
[`docs/SECURITY.md`](https://github.com/eddmpython/xlpod/blob/main/docs/SECURITY.md)
for the full model.

## License

Apache-2.0
