"""Plain dataclasses for response payloads.

Mirrors ``proto/xlpod.openapi.yaml#/components/schemas``. We do not pull
in pydantic — pure stdlib keeps the wheel small and the Pyodide install
fast.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import List


@dataclass(frozen=True)
class Health:
    status: str
    launcher: str
    proto: int


@dataclass(frozen=True)
class Version:
    launcher: str
    proto: int


@dataclass(frozen=True)
class Handshake:
    token: str
    granted_scopes: List[str] = field(default_factory=list)
    granted_fs_roots: List[str] = field(default_factory=list)
    expires_in: int = 0


@dataclass(frozen=True)
class RunResult:
    """Result of a ``/run/python`` call.

    A Python-level exception inside the snippet sets ``ok=False`` and
    populates ``error`` with the traceback; the HTTP status is still
    200 in that case. Worker-level failures (spawn, timeout, crash)
    raise the corresponding ``XlpodError`` subclass instead.
    """

    ok: bool
    stdout: str
    stderr: str
    result: object  # str | None
    error: object  # str | None


@dataclass(frozen=True)
class FileContent:
    """Result of a successful ``/fs/read`` call.

    ``content_bytes`` is the decoded payload; ``content`` retains the
    raw base64 string for callers that need to forward it (e.g. to a
    JSON-only sink) without re-encoding.
    """

    path: str
    size: int
    encoding: str
    content: str
    content_bytes: bytes
