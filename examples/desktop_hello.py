"""End-to-end CPython demo against a running xlpod launcher.

What this exercises:
    1. ``xlpod.Client`` (sync) constructs an HTTPS connection
    2. ``/health``       — anonymous liveness probe
    3. ``/auth/handshake`` — issues a real bearer token
    4. ``/launcher/version`` — bearer-authenticated round trip
    5. Audit log under ``%LOCALAPPDATA%/xlpod/audit.log`` records
       every call with the token id (first 8 hex chars) but not the
       token itself

Prereqs:
    * The launcher is running. From the repo root, run one of:
          cargo run -p xlpod-server      # plain server
          cargo run -p xlpod-launcher    # tray + server
    * mkcert local CA is installed in the user trust store (Phase 0
      did this once). Httpx by default uses certifi, which does not
      know about the Windows trust store, so we point it at the mkcert
      root explicitly via the XLPOD_CA_BUNDLE env var or the mkcert
      CAROOT path. The example tries both.

Run:
    uv run python examples/desktop_hello.py
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

import xlpod


def find_mkcert_root() -> Path | None:
    """Locate the mkcert root CA so httpx can verify the launcher cert."""
    explicit = os.environ.get("XLPOD_CA_BUNDLE")
    if explicit and Path(explicit).exists():
        return Path(explicit)

    mkcert = shutil.which("mkcert")
    if mkcert is not None:
        try:
            out = subprocess.run(
                [mkcert, "-CAROOT"], capture_output=True, text=True, check=True
            )
            caroot = Path(out.stdout.strip())
            candidate = caroot / "rootCA.pem"
            if candidate.exists():
                return candidate
        except (subprocess.CalledProcessError, OSError):
            pass

    # Common fallback location on Windows.
    local = os.environ.get("LOCALAPPDATA")
    if local:
        candidate = Path(local) / "mkcert" / "rootCA.pem"
        if candidate.exists():
            return candidate

    return None


def main() -> int:
    print("xlpod end-to-end demo (CPython sync client)")
    print("-" * 60)

    ca_bundle = find_mkcert_root()
    if ca_bundle is None:
        print("warning: mkcert root CA not found — falling back to verify=False")
        print("         install mkcert or set XLPOD_CA_BUNDLE to silence this")
        verify: object = False
    else:
        print(f"using CA bundle: {ca_bundle}")
        verify = str(ca_bundle)

    repo_root = Path(__file__).resolve().parents[1]

    try:
        with xlpod.Client(verify=verify) as client:
            health = client.health()
            print(f"  health    -> status={health.status} launcher={health.launcher} proto={health.proto}")

            handshake = client.handshake(
                scopes=[
                    "fs:read",
                    "run:python",
                    "excel:com",
                    "ai:provider:call",
                ],
                fs_roots=[str(repo_root)],
            )
            token_id = handshake.token[:8]
            print(f"  handshake -> token_id={token_id}.. scopes={handshake.granted_scopes} roots={len(handshake.granted_fs_roots)} expires_in={handshake.expires_in}")

            version = client.version()
            print(f"  version   -> launcher={version.launcher} proto={version.proto}")

            # Read a real file from the repo to prove the fs:read path
            # works end to end. README.md is the obvious choice.
            target = repo_root / "README.md"
            if not target.exists():
                target = repo_root / "client" / "README.md"
            content = client.read_file(str(target))
            preview = content.content_bytes[:60].decode("utf-8", errors="replace")
            print(f"  fs.read   -> path={Path(content.path).name} size={content.size} preview={preview!r}")

            # Drive the launcher's Python worker through /run/python.
            # The snippet computes a value, prints, and surfaces it
            # via the _result convention.
            run = client.run_python(
                "import sys\n"
                "print('python', sys.version_info[:3])\n"
                "_result = sum(range(10))"
            )
            print(f"  run.python-> ok={run.ok} result={run.result} stdout={run.stdout.strip()!r}")

            # Excel COM is best-effort: pywin32 may be missing or
            # Excel may not be open. Both surface as a specific
            # exception so we can continue without failing the demo.
            try:
                wbs = client.list_workbooks()
                summary = ", ".join(w.name for w in wbs) or "(none open)"
                print(f"  excel.wbs -> count={len(wbs)} {summary}")
            except xlpod.ExcelNotAvailable:
                print("  excel.wbs -> SKIP: pywin32 not in worker python")
            except xlpod.ExcelNotRunning:
                print("  excel.wbs -> SKIP: Excel is not running")

            # Phase 8 — AI bridge. The launcher accepts any
            # ai:provider:call token and the consent dialog (Phase 4
            # mechanism) gates the actual call. With no Anthropic key
            # in the keychain, /ai/chat returns ai_provider_unconfigured
            # — we treat that as a successful demo of the wire.
            try:
                providers = client.list_providers()
                names = ", ".join(
                    f"{p.name}({'set' if p.has_key else 'no-key'})" for p in providers
                )
                print(f"  ai.providers -> {names}")

                session = client.open_session()
                print(
                    f"  ai.session   -> id={session.session_id[:8]}.. "
                    f"model={session.model} scopes={len(session.granted_scopes)}"
                )
                # Try a chat — will likely fail with provider_unconfigured
                # unless ANTHROPIC_API_KEY was injected via set_provider_key.
                ak = os.environ.get("ANTHROPIC_API_KEY")
                if ak:
                    client.set_provider_key(provider="anthropic", key=ak)
                try:
                    resp = client.chat(
                        session_id=session.session_id,
                        messages=[
                            {
                                "role": "user",
                                "content": [
                                    {
                                        "type": "text",
                                        "text": "Say 'xlpod hello' in 3 words.",
                                    }
                                ],
                            }
                        ],
                        max_tokens=50,
                    )
                    text = ""
                    for block in resp.message.get("content", []):
                        if block.get("type") == "text":
                            text = block.get("text", "")
                            break
                    print(f"  ai.chat      -> ok stop={resp.stop_reason} text={text!r}")
                except xlpod.AIProviderUnconfigured:
                    print("  ai.chat      -> SKIP: no anthropic key (set ANTHROPIC_API_KEY)")
            except xlpod.XlpodError as e:
                print(f"  ai           -> SKIP: {type(e).__name__}: {e}")
    except xlpod.LauncherUnreachable as e:
        print(f"FAIL: {e}", file=sys.stderr)
        print("hint: start the launcher with `cargo run -p xlpod-server`", file=sys.stderr)
        return 2
    except xlpod.XlpodError as e:
        print(f"FAIL: {type(e).__name__}: {e}", file=sys.stderr)
        return 1

    print("-" * 60)
    print("OK -- end-to-end round trip succeeded.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
