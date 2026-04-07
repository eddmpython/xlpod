"""xlpod — pure-python client for the xlpod loopback launcher.

See ``proto/xlpod.openapi.yaml`` in the source repo for the
authoritative API surface. The client is intentionally a thin shell
over that spec.
"""

from __future__ import annotations

from ._proto import DEFAULT_BASE_URL, DEFAULT_ORIGIN, PROTO
from .client import AsyncClient, Client
from .errors import (
    AIConsentDenied,
    AIPlanOnlyViolation,
    AIProviderUnconfigured,
    AIProviderUpstream,
    AISessionNotFound,
    AIToolDenied,
    BadRequest,
    ConsentDenied,
    ExcelFailed,
    ExcelNotAvailable,
    ExcelNotRunning,
    ForbiddenPath,
    HostNotAllowed,
    LauncherUnreachable,
    NotAFile,
    OriginNotAllowed,
    PathNotFound,
    PathTooLarge,
    RateLimited,
    ReservedScope,
    ScopeDenied,
    Unauthorized,
    WorkerCrashed,
    WorkerSpawnFailed,
    WorkerTimeout,
    XlpodError,
)
from .models import (
    AISession,
    ChatResponse,
    FileContent,
    Handshake,
    Health,
    ProviderInfo,
    RangeData,
    RunResult,
    Version,
    Workbook,
)

__version__ = "0.0.0"

__all__ = [
    "AIConsentDenied",
    "AIPlanOnlyViolation",
    "AIProviderUnconfigured",
    "AIProviderUpstream",
    "AISession",
    "AISessionNotFound",
    "AIToolDenied",
    "AsyncClient",
    "BadRequest",
    "ChatResponse",
    "Client",
    "ConsentDenied",
    "DEFAULT_BASE_URL",
    "DEFAULT_ORIGIN",
    "ExcelFailed",
    "ExcelNotAvailable",
    "ExcelNotRunning",
    "FileContent",
    "ForbiddenPath",
    "Handshake",
    "Health",
    "HostNotAllowed",
    "LauncherUnreachable",
    "NotAFile",
    "OriginNotAllowed",
    "PROTO",
    "PathNotFound",
    "PathTooLarge",
    "ProviderInfo",
    "RangeData",
    "RateLimited",
    "ReservedScope",
    "RunResult",
    "ScopeDenied",
    "Unauthorized",
    "Version",
    "Workbook",
    "WorkerCrashed",
    "WorkerSpawnFailed",
    "WorkerTimeout",
    "XlpodError",
    "__version__",
]
