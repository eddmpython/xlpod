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
    expires_in: int = 0
