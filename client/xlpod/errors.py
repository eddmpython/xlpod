"""Exception hierarchy.

Mirrors ``proto/xlpod.openapi.yaml#/components/schemas/Error``. Each
specific code maps to a subclass; unknown codes fall back to
``XlpodError``. Callers should ``except XlpodError`` for the catch-all
and the specific subclasses for narrow handling.
"""

from __future__ import annotations

from typing import Any, Mapping


class XlpodError(Exception):
    """Base for every error raised by the xlpod client."""

    def __init__(self, message: str, *, code: str | None = None, hint: str | None = None) -> None:
        super().__init__(message)
        self.code = code
        self.hint = hint

    def __str__(self) -> str:  # pragma: no cover - trivial
        base = super().__str__()
        if self.hint:
            return f"{base} (hint: {self.hint})"
        return base


class LauncherUnreachable(XlpodError):
    """The launcher is not running, or TLS handshake failed."""


class OriginNotAllowed(XlpodError):
    """The configured ``origin`` is not in the launcher's allow-list."""


class HostNotAllowed(XlpodError):
    """The launcher rejected the ``Host`` header (DNS rebinding defense)."""


class Unauthorized(XlpodError):
    """Missing or invalid bearer token. Call ``handshake()`` first."""


class ScopeDenied(XlpodError):
    """The token does not carry a scope required by this route."""


class RateLimited(XlpodError):
    """Per-token rate limit exceeded (default 100 req/s/token)."""


class ReservedScope(XlpodError):
    """A scope from the reserved AI set was requested before its phase."""


class BadRequest(XlpodError):
    """Malformed request payload."""


class ForbiddenPath(XlpodError):
    """Requested path is outside the token's approved fs roots."""


class PathTooLarge(XlpodError):
    """File exceeds the launcher's read size cap (Phase 3: 10 MiB)."""


class NotAFile(XlpodError):
    """Path exists but is a directory, FIFO, device, or socket."""


class PathNotFound(XlpodError):
    """Path does not exist."""


_CODE_MAP: dict[str, type[XlpodError]] = {
    "origin_not_allowed": OriginNotAllowed,
    "host_not_allowed": HostNotAllowed,
    "unauthorized": Unauthorized,
    "scope_denied": ScopeDenied,
    "rate_limited": RateLimited,
    "reserved_scope": ReservedScope,
    "bad_request": BadRequest,
    "forbidden_path": ForbiddenPath,
    "path_too_large": PathTooLarge,
    "not_a_file": NotAFile,
    "path_not_found": PathNotFound,
}


def from_error_body(body: Mapping[str, Any]) -> XlpodError:
    """Construct the right exception from a parsed JSON error body."""
    code = body.get("code")
    message = body.get("message", "request failed")
    hint = body.get("hint")
    cls = _CODE_MAP.get(code, XlpodError) if isinstance(code, str) else XlpodError
    return cls(message, code=code if isinstance(code, str) else None, hint=hint if isinstance(hint, str) else None)
