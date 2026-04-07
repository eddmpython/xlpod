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
import base64
import sys
from typing import Any, Iterable, Optional, Sequence

from . import errors
from ._proto import (
    DEFAULT_BASE_URL,
    DEFAULT_ORIGIN,
    DEFAULT_TIMEOUT_SECONDS,
    HEADER_PROTO,
    PROTO,
)
from ._transport import Transport, TransportResponse, autodetect
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
        verify: Any = True,
        timeout: float = DEFAULT_TIMEOUT_SECONDS,
        transport: Optional[Transport] = None,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._origin = origin
        self._transport: Transport = (
            transport if transport is not None else autodetect(verify=verify, timeout=timeout)
        )
        self._token: Optional[str] = None

    # ---- public API ------------------------------------------------------

    @property
    def token(self) -> Optional[str]:
        """The bearer token issued by the most recent ``handshake()``."""
        return self._token

    async def health(self) -> Health:
        data = await self._request("GET", "/health", auth=False)
        return Health(status=data["status"], launcher=data["launcher"], proto=data["proto"])

    async def handshake(
        self,
        *,
        scopes: Iterable[str],
        fs_roots: Optional[Sequence[str]] = None,
    ) -> Handshake:
        body: dict[str, Any] = {"requested_scopes": list(scopes)}
        if fs_roots is not None:
            body["fs_roots"] = list(fs_roots)
        data = await self._request("POST", "/auth/handshake", json_body=body, auth=False)
        h = Handshake(
            token=data["token"],
            granted_scopes=list(data.get("granted_scopes", [])),
            granted_fs_roots=list(data.get("granted_fs_roots", [])),
            expires_in=int(data.get("expires_in", 0)),
        )
        self._token = h.token
        return h

    async def version(self) -> Version:
        data = await self._request("GET", "/launcher/version", auth=True)
        return Version(launcher=data["launcher"], proto=data["proto"])

    async def run_python(self, code: str) -> RunResult:
        """Execute a Python snippet inside the launcher's worker.

        Requires the ``run:python`` scope. Snippets that raise return a
        ``RunResult`` with ``ok=False`` and the traceback in ``error``;
        worker-level failures (spawn, timeout, crash) raise the
        matching ``XlpodError`` subclass instead.

        Convention: a snippet may set the ``_result`` global, in which
        case its ``repr()`` lands in ``RunResult.result``.
        """
        data = await self._request(
            "POST",
            "/run/python",
            json_body={"code": code},
            auth=True,
        )
        return RunResult(
            ok=bool(data.get("ok", False)),
            stdout=str(data.get("stdout", "")),
            stderr=str(data.get("stderr", "")),
            result=data.get("result"),
            error=data.get("error"),
        )

    async def list_workbooks(self) -> List[Workbook]:
        """List workbooks open in the running Excel instance.

        Requires the ``excel:com`` scope. Raises ``ExcelNotAvailable``
        if the worker's Python lacks ``pywin32``, or ``ExcelNotRunning``
        if Excel is not currently open.
        """
        data = await self._request("GET", "/excel/workbooks", auth=True)
        raw = data.get("workbooks") or []
        out: List[Workbook] = []
        for entry in raw:
            if not isinstance(entry, dict):
                continue
            out.append(
                Workbook(
                    name=str(entry.get("name", "")),
                    path=str(entry.get("path", "")),
                    full_name=str(entry.get("full_name", "")),
                )
            )
        return out

    async def read_range(
        self, *, workbook: str, sheet: str, range: str
    ) -> RangeData:
        """Read a range from an open workbook via Excel COM."""
        body = {"workbook": workbook, "sheet": sheet, "range": range}
        data = await self._request("POST", "/excel/range/read", json_body=body, auth=True)
        raw_values = data.get("values") or []
        normalized: List[List[object]] = []
        for row in raw_values:
            if isinstance(row, list):
                normalized.append(list(row))
            else:
                normalized.append([row])
        return RangeData(
            address=str(data.get("address", "")),
            values=normalized,
        )

    async def open_session(
        self,
        *,
        provider: str = "anthropic",
        model: Optional[str] = None,
    ) -> AISession:
        """Open a logical AI session against the launcher.

        The returned session id ties the chat history together and
        carries the intersection of the user token's scopes with
        the AI tool registry. Phase 8 ships a single Anthropic
        provider; later phases add OpenAI / Ollama.
        """
        body: dict[str, Any] = {"provider": provider}
        if model is not None:
            body["model"] = model
        data = await self._request("POST", "/ai/session", json_body=body, auth=True)
        return AISession(
            session_id=str(data.get("session_id", "")),
            provider=str(data.get("provider", provider)),
            model=str(data.get("model", model or "")),
            granted_scopes=list(data.get("granted_scopes", [])),
            opened_ms=int(data.get("opened_ms", 0)),
        )

    async def chat(
        self,
        *,
        session_id: str,
        messages: list,
        max_tokens: Optional[int] = None,
        plan_only: bool = False,
    ) -> ChatResponse:
        """Send a chat completion through the launcher.

        ``messages`` is a list of dicts in the launcher's
        ``ChatMessage`` shape (``{"role": ..., "content": [...]}``);
        for Phase 8 we keep this loose so callers building messages
        from a notebook can pass them through unchanged.
        """
        body: dict[str, Any] = {
            "session_id": session_id,
            "messages": messages,
            "plan_only": plan_only,
        }
        if max_tokens is not None:
            body["max_tokens"] = max_tokens
        data = await self._request("POST", "/ai/chat", json_body=body, auth=True)
        return ChatResponse(
            session_id=str(data.get("session_id", session_id)),
            message=dict(data.get("message", {})),
            stop_reason=str(data.get("stop_reason", "end_turn")),
            usage=dict(data.get("usage", {})),
        )

    async def get_session_history(self, session_id: str) -> list:
        """Return the full transcript for an AI session."""
        data = await self._request(
            "GET", f"/ai/session/{session_id}/history", auth=True
        )
        return list(data.get("messages", []))

    async def list_providers(self) -> list[ProviderInfo]:
        data = await self._request("GET", "/ai/providers", auth=True)
        return [
            ProviderInfo(name=str(p.get("name", "")), has_key=bool(p.get("has_key", False)))
            for p in data.get("providers", [])
        ]

    async def set_provider_key(self, *, provider: str, key: str) -> None:
        """Store an AI provider key in the launcher's keychain.

        This call always opens a consent dialog on the launcher
        side; the user must approve. The key is never echoed.
        """
        body = {"provider": provider, "key": key}
        await self._request("POST", "/ai/providers/key", json_body=body, auth=True)

    async def delete_provider_key(self, *, provider: str) -> None:
        await self._request(
            "DELETE",
            "/ai/providers/key",
            query={"provider": provider},
            auth=True,
        )

    async def get_cost_today(self) -> dict:
        """Return today's AI usage rollup (Phase 9).

        Cost is reported as integer micro-USD; divide by 1_000_000
        for dollars.
        """
        return await self._request("GET", "/ai/cost/today", auth=True)

    async def open_trust_window(
        self,
        *,
        session_id: str,
        tools: list,
        duration_secs: int,
    ) -> dict:
        """Open a trust window so the launcher skips per-call consent
        for the named tools for ``duration_secs`` seconds.
        """
        body = {
            "session_id": session_id,
            "tools": list(tools),
            "duration_secs": int(duration_secs),
        }
        return await self._request(
            "POST", "/ai/consent/window", json_body=body, auth=True
        )

    async def read_file(self, path: str) -> FileContent:
        """Read a file under one of the token's approved fs roots.

        Requires the ``fs:read`` scope and at least one ``fs_roots``
        entry attached to the token at handshake time. The launcher
        canonicalizes the path, verifies it lies under an approved
        root, and rejects directories, oversized files, and missing
        paths with the corresponding ``XlpodError`` subclass.
        """
        data = await self._request(
            "GET",
            "/fs/read",
            query={"path": path},
            auth=True,
        )
        encoding = str(data.get("encoding", ""))
        raw = str(data.get("content", ""))
        if encoding == "base64":
            content_bytes = base64.b64decode(raw)
        else:
            # Forward-compatible: future encodings stay openable as a
            # raw string for callers that recognize them.
            content_bytes = raw.encode("utf-8")
        return FileContent(
            path=str(data.get("path", "")),
            size=int(data.get("size", 0)),
            encoding=encoding,
            content=raw,
            content_bytes=content_bytes,
        )

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
        query: Optional[dict[str, str]] = None,
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
        if query:
            from urllib.parse import urlencode

            url = f"{url}?{urlencode(query)}"
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
    """Synchronous wrapper around :class:`AsyncClient`. CPython only.

    The wrapper owns one persistent ``asyncio`` event loop for its
    entire lifetime — *not* a fresh ``asyncio.run`` per call. This
    matters on Windows: ``httpx.AsyncClient`` binds its connection pool
    and TLS state to whichever loop first touched it, and the
    ``ProactorEventLoop`` raises ``Event loop is closed`` if a later
    call uses a different loop. One loop, one client, no surprises.

    If you are already inside an event loop, use :class:`AsyncClient`
    directly instead.
    """

    def __init__(
        self,
        *,
        base_url: str = DEFAULT_BASE_URL,
        origin: str = DEFAULT_ORIGIN,
        verify: Any = True,
        timeout: float = DEFAULT_TIMEOUT_SECONDS,
        transport: Optional[Transport] = None,
    ) -> None:
        if sys.platform == "emscripten":
            raise RuntimeError(
                "xlpod.Client is sync; use xlpod.AsyncClient on Pyodide / xlwings Lite"
            )
        self._loop: Optional[asyncio.AbstractEventLoop] = asyncio.new_event_loop()
        self._async = AsyncClient(
            base_url=base_url,
            origin=origin,
            verify=verify,
            timeout=timeout,
            transport=transport,
        )

    # ---- public API ------------------------------------------------------

    @property
    def token(self) -> Optional[str]:
        return self._async.token

    def health(self) -> Health:
        return self._run(self._async.health())

    def handshake(
        self,
        *,
        scopes: Iterable[str],
        fs_roots: Optional[Sequence[str]] = None,
    ) -> Handshake:
        return self._run(self._async.handshake(scopes=scopes, fs_roots=fs_roots))

    def version(self) -> Version:
        return self._run(self._async.version())

    def read_file(self, path: str) -> FileContent:
        return self._run(self._async.read_file(path))

    def run_python(self, code: str) -> RunResult:
        return self._run(self._async.run_python(code))

    def list_workbooks(self) -> List[Workbook]:
        return self._run(self._async.list_workbooks())

    def read_range(self, *, workbook: str, sheet: str, range: str) -> RangeData:
        return self._run(
            self._async.read_range(workbook=workbook, sheet=sheet, range=range)
        )

    def open_session(
        self,
        *,
        provider: str = "anthropic",
        model: Optional[str] = None,
    ) -> AISession:
        return self._run(self._async.open_session(provider=provider, model=model))

    def chat(
        self,
        *,
        session_id: str,
        messages: list,
        max_tokens: Optional[int] = None,
        plan_only: bool = False,
    ) -> ChatResponse:
        return self._run(
            self._async.chat(
                session_id=session_id,
                messages=messages,
                max_tokens=max_tokens,
                plan_only=plan_only,
            )
        )

    def get_session_history(self, session_id: str) -> list:
        return self._run(self._async.get_session_history(session_id))

    def list_providers(self) -> list[ProviderInfo]:
        return self._run(self._async.list_providers())

    def set_provider_key(self, *, provider: str, key: str) -> None:
        self._run(self._async.set_provider_key(provider=provider, key=key))

    def delete_provider_key(self, *, provider: str) -> None:
        self._run(self._async.delete_provider_key(provider=provider))

    def get_cost_today(self) -> dict:
        return self._run(self._async.get_cost_today())

    def open_trust_window(
        self,
        *,
        session_id: str,
        tools: list,
        duration_secs: int,
    ) -> dict:
        return self._run(
            self._async.open_trust_window(
                session_id=session_id,
                tools=tools,
                duration_secs=duration_secs,
            )
        )

    def close(self) -> None:
        if self._loop is None:
            return
        try:
            self._loop.run_until_complete(self._async.aclose())
        finally:
            self._loop.close()
            self._loop = None

    def __enter__(self) -> "Client":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    def __del__(self) -> None:  # best-effort cleanup if user forgot close()
        try:
            self.close()
        except Exception:
            pass

    def _run(self, coro: Any) -> Any:
        if self._loop is None:
            raise RuntimeError("xlpod.Client has been closed")
        return self._loop.run_until_complete(coro)
