"""xlpod — pure-python client for the xlpod loopback launcher.

See ``proto/xlpod.openapi.yaml`` in the source repo for the
authoritative API surface. The client is intentionally a thin shell
over that spec.
"""

from __future__ import annotations

from ._proto import DEFAULT_BASE_URL, DEFAULT_ORIGIN, PROTO
from .client import AsyncClient, Client
from .errors import (
    BadRequest,
    HostNotAllowed,
    LauncherUnreachable,
    OriginNotAllowed,
    RateLimited,
    ReservedScope,
    ScopeDenied,
    Unauthorized,
    XlpodError,
)
from .models import Handshake, Health, Version

__version__ = "0.0.0"

__all__ = [
    "AsyncClient",
    "BadRequest",
    "Client",
    "DEFAULT_BASE_URL",
    "DEFAULT_ORIGIN",
    "Handshake",
    "Health",
    "HostNotAllowed",
    "LauncherUnreachable",
    "OriginNotAllowed",
    "PROTO",
    "RateLimited",
    "ReservedScope",
    "ScopeDenied",
    "Unauthorized",
    "Version",
    "XlpodError",
    "__version__",
]
