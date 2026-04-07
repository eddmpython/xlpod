"""
Phase 0 CSP probe — go/no-go experiment for the entire xlpod project.

Goal: determine whether xlwings Lite (Pyodide, running inside an Office Add-in
iframe) can execute `fetch('https://127.0.0.1:7421/health')` against a local
HTTPS loopback server. The answer is gated by the iframe's Content-Security-
Policy `connect-src` directive — which is not documented and must be measured.

This script:
  1. Generates a self-signed certificate for 127.0.0.1 + ::1 (in-memory ->
     temp files) using `cryptography`. No system trust store changes.
  2. Starts a minimal HTTPS server on https://127.0.0.1:7421 exposing GET
     /health only. No auth, no other routes — this is a measurement tool,
     not the real launcher.
  3. Prints the exact DevTools snippet the user must paste into the Lite
     workbook console.

Run:
    uv sync --group phase0
    uv run --group phase0 python scripts/phase0_csp_probe/probe.py

Then open xlwings Lite, F12, paste the snippet, record the result in
docs/phase0-report.md.

This file MUST stay loopback-only and MUST NOT be evolved into the real
launcher. The real launcher is Rust + axum + rustls.
"""

from __future__ import annotations

import datetime as dt
import ipaddress
import ssl
import tempfile
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID
from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
import uvicorn

HOST = "127.0.0.1"
PORT = 7421
# Probe-only: we *measure* CSP behavior. The real launcher will use a strict
# origin whitelist resolved from Phase 0 results, not "*".
PROBE_ALLOWED_ORIGINS = ["*"]


def generate_selfsigned_cert(out_dir: Path) -> tuple[Path, Path]:
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "xlpod-phase0-probe")])
    san = x509.SubjectAlternativeName(
        [
            x509.DNSName("localhost"),
            x509.IPAddress(ipaddress.IPv4Address("127.0.0.1")),
            x509.IPAddress(ipaddress.IPv6Address("::1")),
        ]
    )
    now = dt.datetime.now(dt.UTC)
    cert = (
        x509.CertificateBuilder()
        .subject_name(name)
        .issuer_name(name)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - dt.timedelta(minutes=1))
        .not_valid_after(now + dt.timedelta(days=1))
        .add_extension(san, critical=False)
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .sign(key, hashes.SHA256())
    )

    cert_path = out_dir / "probe-cert.pem"
    key_path = out_dir / "probe-key.pem"
    cert_path.write_bytes(cert.public_bytes(serialization.Encoding.PEM))
    key_path.write_bytes(
        key.private_bytes(
            encoding=serialization.Encoding.PEM,
            format=serialization.PrivateFormat.TraditionalOpenSSL,
            encryption_algorithm=serialization.NoEncryption(),
        )
    )
    return cert_path, key_path


def build_app() -> FastAPI:
    app = FastAPI(title="xlpod Phase 0 probe", docs_url=None, redoc_url=None)
    app.add_middleware(
        CORSMiddleware,
        allow_origins=PROBE_ALLOWED_ORIGINS,
        allow_credentials=False,
        allow_methods=["GET", "OPTIONS"],
        allow_headers=["*"],
    )

    @app.get("/health")
    async def health() -> dict[str, str]:
        return {"status": "ok", "probe": "phase0", "proto": "0"}

    return app


REPO_ROOT = Path(__file__).resolve().parents[2]
MKCERT_DIR = REPO_ROOT / ".certs"
MKCERT_CERT = MKCERT_DIR / "probe-cert.pem"
MKCERT_KEY = MKCERT_DIR / "probe-key.pem"


def main() -> None:
    if MKCERT_CERT.exists() and MKCERT_KEY.exists():
        cert_path, key_path = MKCERT_CERT, MKCERT_KEY
        cert_kind = "mkcert (trusted)"
    else:
        tmp = Path(tempfile.mkdtemp(prefix="xlpod-phase0-"))
        cert_path, key_path = generate_selfsigned_cert(tmp)
        cert_kind = "ephemeral self-signed (untrusted — expect ERR_CERT_AUTHORITY_INVALID)"

    print("=" * 72)
    print("xlpod Phase 0 CSP probe")
    print("=" * 72)
    print(f"Cert ({cert_kind}): {cert_path}")
    print(f"Key:  {key_path}")
    print(f"Listening:        https://{HOST}:{PORT}/health  (loopback only)")
    print()
    print("Step 1 — verify locally (a cert error here is EXPECTED):")
    print(f"    curl --insecure https://{HOST}:{PORT}/health")
    print()
    print("Step 2 — open xlwings Lite in Excel, press F12 (DevTools), paste:")
    print()
    print("    // 2a. capture origin + headers")
    print("    console.log('origin:', location.origin);")
    print("    fetch('https://127.0.0.1:7421/health')")
    print("      .then(r => r.text().then(t => console.log('OK', r.status, t)))")
    print("      .catch(e => console.error('FAIL', e));")
    print()
    print("Step 3 — record the result + any CSP violation messages in")
    print("    docs/phase0-report.md  (GREEN / YELLOW / RED).")
    print()
    print("Press Ctrl-C to stop.")
    print("=" * 72)

    ssl_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ssl_ctx.load_cert_chain(certfile=str(cert_path), keyfile=str(key_path))

    uvicorn.run(
        build_app(),
        host=HOST,
        port=PORT,
        ssl_certfile=str(cert_path),
        ssl_keyfile=str(key_path),
        log_level="info",
    )


if __name__ == "__main__":
    main()
