"""Constants derived from ``proto/xlpod.openapi.yaml``.

This module is the *client-side* mirror of the spec. The launcher reads
the same values out of the spec extensions; CI Phase 1.4-followup will
diff them. Until then: when the spec changes, update this file in the
same PR.
"""

from __future__ import annotations

PROTO: int = 1
"""Wire-protocol version. Sent as ``X-XLPod-Proto`` on every request."""

DEFAULT_BASE_URL: str = "https://127.0.0.1:7421"
"""Loopback launcher endpoint. Mirror of ``servers[0].url``."""

DEFAULT_ORIGIN: str = "https://addin.xlwings.org"
"""The single value in ``info.x-xlpod-allowed-origins``. Used by the
CPython transport to set the ``Origin`` header explicitly. In Pyodide
the browser sets ``Origin`` automatically and we leave it alone."""

# Header names — kept here so swaps are spec-driven and case-stable.
HEADER_PROTO: str = "X-XLPod-Proto"
HEADER_PLAN_ONLY: str = "X-XLPod-Plan-Only"

DEFAULT_TIMEOUT_SECONDS: float = 10.0
