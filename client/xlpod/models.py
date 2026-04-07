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
