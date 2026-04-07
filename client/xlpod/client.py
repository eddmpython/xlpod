"""High-level client classes.

Two surfaces:

- ``AsyncClient`` — works on both CPython and Pyodide. Methods are
  ``async``. Use this in xlwings Lite and in any CPython code that is
  already async.
- ``Client`` — sync wrapper around ``AsyncClient``. CPython only;
  raises on Pyodide because there is no sync HTTP path in the browser.
"""

from __future__ import annotations

import asyncio
import sys
from typing import Iterable, List, Optional

from . import errors
from ._proto import (
    DEFAULT_BASE_URL,
    DEFAULT_ORIGIN,
    DEFAULT_TIMEOUT_SECONDS,
    HEADER_PROTO,
    PROTO,
)
from ._transport import Transport, TransportResponse, autodetect
from .models import Handshake, Health, Version


class AsyncClient:
    """Async client for the xlpod loopback launcher.

    Construct with no arguments to talk to the default launcher
    endpoint, or pass ``base_url`` / ``origin`` for a self-hosted
    deployment. Inject a ``transport`` for tests.
    """

    def __init__(
        self,
        *,
        base_url: str = DEFAULT_BASE_URL,
        origin: str = DEFAULT_ORIGIN,
        transport: Optional[Transport] = None,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._origin = origin
        self._transport: Transport = transport if transport is not None else autodetect()
        self._token: Optional[str] = None

    # ---- public API ------------------------------------------------------

    @property
    def token(self) -> Optional[str]:
        """The bearer token issued by the most recent ``handshake()``."""
        return self._token

    async def health(self) -> Health:
        data = await self._request("GET", "/health", auth=False)
        return Health(status=data["status"], launcher=data["launcher"], proto=data["proto"])

    async def handshake(self, *, scopes: Iterable[str]) -> Handshake:
        body = {"requested_scopes": list(scopes)}
        data = await self._request("POST", "/auth/handshake", json_body=body, auth=False)
        h = Handshake(
            token=data["token"],
            granted_scopes=list(data.get("granted_scopes", [])),
            expires_in=int(data.get("expires_in", 0)),
        )
        self._token = h.token
        return h

    async def version(self) -> Version:
        data = await self._request("GET", "/launcher/version", auth=True)
        return Version(launcher=data["launcher"], proto=data["proto"])

    async def aclose(self) -> None:
        await self._transport.aclose()

    async def __aenter__(self) -> "AsyncClient":
        return self

    async def __aexit__(self, *_exc: object) -> None:
        await self.aclose()

    # ---- internals -------------------------------------------------------

    async def _request(
        self,
        method: str,
        path: str,
        *,
        json_body: Optional[object] = None,
        auth: bool,
    ) -> dict:
        headers = {
            HEADER_PROTO: str(PROTO),
            "Origin": self._origin,
        }
        if auth:
            if self._token is None:
                raise errors.Unauthorized("no token; call handshake() first")
            headers["Authorization"] = f"Bearer {self._token}"
        url = self._base_url + path
        resp: TransportResponse = await self._transport.request(
            method, url, headers=headers, json_body=json_body
        )
        if resp.status_code >= 400:
            try:
                body = resp.json() or {}
            except Exception:
                body = {}
            if isinstance(body, dict):
                raise errors.from_error_body(body)
            raise errors.XlpodError(f"HTTP {resp.status_code}")
        data = resp.json()
        if not isinstance(data, dict):
            raise errors.XlpodError(f"unexpected response shape from {path}")
        return data


class Client:
    """Synchronous wrapper. CPython only.

    Each call runs a fresh asyncio event loop via ``asyncio.run``. This
    is fine for the launcher's traffic profile (one user, low rate) and
    keeps the sync surface dependency-free. If you are inside an
    existing event loop, use ``AsyncClient`` directly.
    """

    def __init__(
        self,
        *,
        base_url: str = DEFAULT_BASE_URL,
        origin: str = DEFAULT_ORIGIN,
        transport: Optional[Transport] = None,
    ) -> None:
        if sys.platform == "emscripten":
            raise RuntimeError(
                "xlpod.Client is sync; use xlpod.AsyncClient on Pyodide / xlwings Lite"
            )
        self._async = AsyncClient(base_url=base_url, origin=origin, transport=transport)

    # ---- public API ------------------------------------------------------

    @property
    def token(self) -> Optional[str]:
        return self._async.token

    def health(self) -> Health:
        return asyncio.run(self._async.health())

    def handshake(self, *, scopes: Iterable[str]) -> Handshake:
        return asyncio.run(self._async.handshake(scopes=scopes))

    def version(self) -> Version:
        return asyncio.run(self._async.version())

    def close(self) -> None:
        asyncio.run(self._async.aclose())

    def __enter__(self) -> "Client":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()
