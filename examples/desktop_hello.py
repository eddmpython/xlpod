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

    try:
        with xlpod.Client(verify=verify) as client:
            health = client.health()
            print(f"  health    -> status={health.status} launcher={health.launcher} proto={health.proto}")

            handshake = client.handshake(scopes=["fs:read"])
            token_id = handshake.token[:8]
            print(f"  handshake -> token_id={token_id}.. scopes={handshake.granted_scopes} expires_in={handshake.expires_in}")

            version = client.version()
            print(f"  version   -> launcher={version.launcher} proto={version.proto}")
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
