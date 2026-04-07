"""Pluggable HTTP transport.

There are two real backends:

- ``HttpxAsyncTransport`` — wraps ``httpx.AsyncClient``. Default on
  CPython. Sets the ``Origin`` header explicitly because there is no
  browser to do it for us.
- ``PyodideTransport`` — wraps ``pyodide.http.pyfetch``. Default in a
  Pyodide / xlwings Lite environment. Does **not** set the ``Origin``
  header — the browser sets it from the iframe document URL and any
  user override would simply be stripped.

Tests inject a ``FakeTransport`` so they never touch the network.

Every transport returns a small response wrapper with ``.status_code``
and ``.json()``. We do not expose raw httpx / pyfetch types in the
public API to keep the surface stable across backends.
"""

from __future__ import annotations

import json as _json
import sys
from dataclasses import dataclass
from typing import Any, Mapping, Optional, Protocol


@dataclass
class TransportResponse:
    status_code: int
    body: bytes

    def json(self) -> Any:
        if not self.body:
            return None
        return _json.loads(self.body.decode("utf-8"))


class Transport(Protocol):
    """Async request method shared by every backend."""

    async def request(
        self,
        method: str,
        url: str,
        *,
        headers: Mapping[str, str],
        json_body: Optional[Any] = None,
    ) -> TransportResponse: ...

    async def aclose(self) -> None: ...


class HttpxAsyncTransport:
    """CPython transport built on ``httpx.AsyncClient``."""

    def __init__(self, *, verify: Any = True, timeout: float = 10.0) -> None:
        # Imported lazily so the package still imports cleanly on Pyodide
        # where httpx may not be installed.
        import httpx  # noqa: PLC0415

        self._client = httpx.AsyncClient(verify=verify, timeout=timeout)

    async def request(
        self,
        method: str,
        url: str,
        *,
        headers: Mapping[str, str],
        json_body: Optional[Any] = None,
    ) -> TransportResponse:
        import httpx  # noqa: PLC0415

        try:
            resp = await self._client.request(
                method,
                url,
                headers=dict(headers),
                json=json_body,
            )
        except httpx.ConnectError as e:
            from .errors import LauncherUnreachable

            raise LauncherUnreachable(f"could not reach launcher at {url}: {e}") from e
        return TransportResponse(status_code=resp.status_code, body=resp.content)

    async def aclose(self) -> None:
        await self._client.aclose()


class PyodideTransport:
    """Pyodide / xlwings Lite transport built on ``pyodide.http.pyfetch``."""

    def __init__(self) -> None:
        # Imported lazily; failure surfaces only when this transport is
        # actually selected.
        from pyodide.http import pyfetch  # type: ignore[import-not-found]  # noqa: PLC0415

        self._pyfetch = pyfetch

    async def request(
        self,
        method: str,
        url: str,
        *,
        headers: Mapping[str, str],
        json_body: Optional[Any] = None,
    ) -> TransportResponse:
        # Drop the Origin header — the browser sets it from the iframe
        # document and any user override is silently ignored.
        clean_headers = {k: v for k, v in headers.items() if k.lower() != "origin"}
        kwargs: dict[str, Any] = {
            "method": method,
            "headers": clean_headers,
        }
        if json_body is not None:
            kwargs["body"] = _json.dumps(json_body)
            clean_headers.setdefault("Content-Type", "application/json")
        try:
            resp = await self._pyfetch(url, **kwargs)
        except Exception as e:  # pyfetch raises a generic JsException
            from .errors import LauncherUnreachable

            raise LauncherUnreachable(f"could not reach launcher at {url}: {e}") from e
        body = await resp.bytes()
        return TransportResponse(status_code=resp.status, body=body)

    async def aclose(self) -> None:
        return None


def autodetect() -> Transport:
    """Return the right transport for the current environment.

    Pyodide reports ``sys.platform == "emscripten"``. Anywhere else we
    assume CPython and use httpx.
    """
    if sys.platform == "emscripten":
        return PyodideTransport()
    return HttpxAsyncTransport()
