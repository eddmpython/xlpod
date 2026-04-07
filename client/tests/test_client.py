"""Unit tests for ``xlpod.AsyncClient`` and ``xlpod.Client``.

These tests do not touch the network. They use a ``FakeTransport`` that
records every request and returns canned responses, so they exercise
the full request/response code path (header construction, error
mapping, token storage) without depending on a running launcher.

The launcher's own integration tests in
``launcher/xlpod-server/tests/api.rs`` cover the server side; together
they meet end-to-end through the protocol spec.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, List, Mapping, Optional

import pytest

import xlpod
from xlpod._transport import TransportResponse


@dataclass
class _Recorded:
    method: str
    url: str
    headers: dict
    json_body: Optional[Any]


class FakeTransport:
    def __init__(self, responses: List[TransportResponse]) -> None:
        self._responses = list(responses)
        self.recorded: List[_Recorded] = []
        self.closed = False

    async def request(
        self,
        method: str,
        url: str,
        *,
        headers: Mapping[str, str],
        json_body: Optional[Any] = None,
    ) -> TransportResponse:
        self.recorded.append(
            _Recorded(method=method, url=url, headers=dict(headers), json_body=json_body)
        )
        if not self._responses:
            raise AssertionError("FakeTransport ran out of canned responses")
        return self._responses.pop(0)

    async def aclose(self) -> None:
        self.closed = True


def _ok(payload: dict) -> TransportResponse:
    return TransportResponse(status_code=200, body=json.dumps(payload).encode("utf-8"))


def _err(status: int, code: str, message: str = "x", hint: str | None = None) -> TransportResponse:
    body: dict[str, Any] = {"code": code, "message": message}
    if hint:
        body["hint"] = hint
    return TransportResponse(status_code=status, body=json.dumps(body).encode("utf-8"))


# ---- AsyncClient -----------------------------------------------------------


@pytest.mark.asyncio
async def test_health_returns_dataclass() -> None:
    transport = FakeTransport([_ok({"status": "ok", "launcher": "0.0.0", "proto": 1})])
    async with xlpod.AsyncClient(transport=transport) as client:
        h = await client.health()
    assert isinstance(h, xlpod.Health)
    assert h.status == "ok"
    assert h.launcher == "0.0.0"
    assert h.proto == 1
    rec = transport.recorded[0]
    assert rec.method == "GET"
    assert rec.url.endswith("/health")
    assert rec.headers["X-XLPod-Proto"] == "1"
    assert rec.headers["Origin"] == xlpod.DEFAULT_ORIGIN


@pytest.mark.asyncio
async def test_handshake_stores_token_and_then_authorizes_version() -> None:
    transport = FakeTransport(
        [
            _ok(
                {
                    "token": "deadbeef" * 8,
                    "granted_scopes": ["fs:read"],
                    "expires_in": 3600,
                }
            ),
            _ok({"launcher": "0.0.0", "proto": 1}),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    h = await client.handshake(scopes=["fs:read"])
    assert h.token == "deadbeef" * 8
    assert client.token == h.token

    v = await client.version()
    assert isinstance(v, xlpod.Version)
    assert v.proto == 1

    handshake_call, version_call = transport.recorded
    assert handshake_call.json_body == {"requested_scopes": ["fs:read"]}
    assert "Authorization" not in handshake_call.headers
    assert version_call.headers["Authorization"] == f"Bearer {'deadbeef' * 8}"


@pytest.mark.asyncio
async def test_version_without_handshake_raises() -> None:
    transport = FakeTransport([])  # no requests should be made
    client = xlpod.AsyncClient(transport=transport)
    with pytest.raises(xlpod.Unauthorized):
        await client.version()
    assert transport.recorded == []


@pytest.mark.asyncio
async def test_origin_not_allowed_maps_to_specific_exception() -> None:
    transport = FakeTransport([_err(403, "origin_not_allowed", hint="only addin.xlwings.org")])
    client = xlpod.AsyncClient(transport=transport)
    with pytest.raises(xlpod.OriginNotAllowed) as ei:
        await client.handshake(scopes=["fs:read"])
    assert ei.value.code == "origin_not_allowed"
    assert ei.value.hint == "only addin.xlwings.org"


@pytest.mark.asyncio
async def test_reserved_scope_maps_to_specific_exception() -> None:
    transport = FakeTransport([_err(400, "reserved_scope")])
    client = xlpod.AsyncClient(transport=transport)
    with pytest.raises(xlpod.ReservedScope):
        await client.handshake(scopes=["ai:provider:call"])


@pytest.mark.asyncio
async def test_unknown_error_code_falls_back_to_xlpoderror() -> None:
    transport = FakeTransport([_err(500, "something_new", message="oops")])
    client = xlpod.AsyncClient(transport=transport)
    with pytest.raises(xlpod.XlpodError) as ei:
        await client.handshake(scopes=["fs:read"])
    # Catch-all but not one of the specific subclasses
    assert not isinstance(
        ei.value,
        (xlpod.OriginNotAllowed, xlpod.Unauthorized, xlpod.ReservedScope),
    )


@pytest.mark.asyncio
async def test_aclose_closes_transport() -> None:
    transport = FakeTransport([])
    client = xlpod.AsyncClient(transport=transport)
    await client.aclose()
    assert transport.closed is True


# ---- Client (sync wrapper) -------------------------------------------------


def test_sync_client_health_round_trip() -> None:
    transport = FakeTransport([_ok({"status": "ok", "launcher": "0.0.0", "proto": 1})])
    with xlpod.Client(transport=transport) as client:
        h = client.health()
    assert h.status == "ok"
    assert transport.closed is True


def test_sync_client_handshake_then_version() -> None:
    token = "a" * 64
    transport = FakeTransport(
        [
            _ok({"token": token, "granted_scopes": ["fs:read"], "expires_in": 3600}),
            _ok({"launcher": "0.0.0", "proto": 1}),
        ]
    )
    client = xlpod.Client(transport=transport)
    client.handshake(scopes=["fs:read"])
    assert client.token == token
    v = client.version()
    assert v.proto == 1
    client.close()
