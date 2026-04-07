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
async def test_consent_denied_maps_to_specific_exception() -> None:
    transport = FakeTransport([_err(403, "consent_denied", hint="approve in tray")])
    client = xlpod.AsyncClient(transport=transport)
    with pytest.raises(xlpod.ConsentDenied) as ei:
        await client.handshake(scopes=["fs:read"], fs_roots=["/tmp"])
    assert ei.value.code == "consent_denied"


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


@pytest.mark.asyncio
async def test_list_workbooks_parses_response_into_dataclasses() -> None:
    transport = FakeTransport(
        [
            _ok({"token": "a" * 64, "granted_scopes": ["excel:com"], "expires_in": 3600}),
            _ok(
                {
                    "workbooks": [
                        {"name": "Book1.xlsx", "path": "C:/tmp", "full_name": "C:/tmp/Book1.xlsx"},
                        {"name": "Empty.xlsx", "path": "", "full_name": "Empty.xlsx"},
                    ]
                }
            ),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    await client.handshake(scopes=["excel:com"])
    wbs = await client.list_workbooks()
    assert len(wbs) == 2
    assert isinstance(wbs[0], xlpod.Workbook)
    assert wbs[0].name == "Book1.xlsx"
    assert wbs[1].path == ""


@pytest.mark.asyncio
async def test_list_workbooks_excel_not_available_maps_to_specific_exception() -> None:
    transport = FakeTransport(
        [
            _ok({"token": "b" * 64, "granted_scopes": ["excel:com"], "expires_in": 3600}),
            _err(503, "excel_not_available", hint="pip install pywin32"),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    await client.handshake(scopes=["excel:com"])
    with pytest.raises(xlpod.ExcelNotAvailable):
        await client.list_workbooks()


@pytest.mark.asyncio
async def test_read_range_returns_2d_values_and_address() -> None:
    transport = FakeTransport(
        [
            _ok({"token": "c" * 64, "granted_scopes": ["excel:com"], "expires_in": 3600}),
            _ok(
                {
                    "address": "$A$1:$B$2",
                    "values": [[1, "two"], [3.0, None]],
                }
            ),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    await client.handshake(scopes=["excel:com"])
    rng = await client.read_range(workbook="Book1.xlsx", sheet="Sheet1", range="A1:B2")
    assert isinstance(rng, xlpod.RangeData)
    assert rng.address == "$A$1:$B$2"
    assert rng.values == [[1, "two"], [3.0, None]]


@pytest.mark.asyncio
async def test_read_range_excel_not_running_maps_to_specific_exception() -> None:
    transport = FakeTransport(
        [
            _ok({"token": "d" * 64, "granted_scopes": ["excel:com"], "expires_in": 3600}),
            _err(503, "excel_not_running"),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    await client.handshake(scopes=["excel:com"])
    with pytest.raises(xlpod.ExcelNotRunning):
        await client.read_range(workbook="Book1.xlsx", sheet="Sheet1", range="A1")


@pytest.mark.asyncio
async def test_run_python_happy_returns_result_dataclass() -> None:
    transport = FakeTransport(
        [
            _ok({"token": "9" * 64, "granted_scopes": ["run:python"], "expires_in": 3600}),
            _ok(
                {
                    "ok": True,
                    "stdout": "hi\n",
                    "stderr": "",
                    "result": "3",
                    "error": None,
                }
            ),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    await client.handshake(scopes=["run:python"])
    result = await client.run_python("print('hi'); _result = 1+2")
    assert isinstance(result, xlpod.RunResult)
    assert result.ok is True
    assert result.stdout == "hi\n"
    assert result.result == "3"
    assert result.error is None
    run_call = transport.recorded[1]
    assert run_call.json_body == {"code": "print('hi'); _result = 1+2"}
    assert run_call.headers["Authorization"] == f"Bearer {'9' * 64}"


@pytest.mark.asyncio
async def test_run_python_python_level_exception_returns_ok_false() -> None:
    transport = FakeTransport(
        [
            _ok({"token": "8" * 64, "granted_scopes": ["run:python"], "expires_in": 3600}),
            _ok(
                {
                    "ok": False,
                    "stdout": "",
                    "stderr": "",
                    "result": None,
                    "error": "Traceback...\nZeroDivisionError: division by zero",
                }
            ),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    await client.handshake(scopes=["run:python"])
    # NB: this does NOT raise — Python exceptions inside the snippet
    # come back as ok=False, not as XlpodError.
    result = await client.run_python("1/0")
    assert result.ok is False
    assert "ZeroDivisionError" in str(result.error)


@pytest.mark.asyncio
async def test_run_python_worker_timeout_raises_specific_exception() -> None:
    transport = FakeTransport(
        [
            _ok({"token": "7" * 64, "granted_scopes": ["run:python"], "expires_in": 3600}),
            _err(504, "worker_timeout"),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    await client.handshake(scopes=["run:python"])
    with pytest.raises(xlpod.WorkerTimeout):
        await client.run_python("import time; time.sleep(60)")


@pytest.mark.asyncio
async def test_read_file_decodes_base64_and_records_query() -> None:
    import base64

    payload = b"hello, xlpod"
    encoded = base64.b64encode(payload).decode("ascii")
    transport = FakeTransport(
        [
            _ok(
                {
                    "token": "f" * 64,
                    "granted_scopes": ["fs:read"],
                    "granted_fs_roots": ["/tmp"],
                    "expires_in": 3600,
                }
            ),
            _ok(
                {
                    "path": "/tmp/hello.txt",
                    "size": len(payload),
                    "encoding": "base64",
                    "content": encoded,
                }
            ),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    h = await client.handshake(scopes=["fs:read"], fs_roots=["/tmp"])
    assert h.granted_fs_roots == ["/tmp"]

    file = await client.read_file("/tmp/hello.txt")
    assert isinstance(file, xlpod.FileContent)
    assert file.size == 12
    assert file.content_bytes == payload
    assert file.encoding == "base64"

    handshake_call, read_call = transport.recorded
    assert handshake_call.json_body == {
        "requested_scopes": ["fs:read"],
        "fs_roots": ["/tmp"],
    }
    assert "/fs/read?path=" in read_call.url
    assert read_call.headers["Authorization"] == f"Bearer {'f' * 64}"


@pytest.mark.asyncio
async def test_read_file_forbidden_path_maps_to_specific_exception() -> None:
    transport = FakeTransport(
        [
            _ok(
                {
                    "token": "1" * 64,
                    "granted_scopes": ["fs:read"],
                    "granted_fs_roots": ["/tmp"],
                    "expires_in": 3600,
                }
            ),
            _err(403, "forbidden_path", hint="widen fs_roots"),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    await client.handshake(scopes=["fs:read"], fs_roots=["/tmp"])
    with pytest.raises(xlpod.ForbiddenPath) as ei:
        await client.read_file("/etc/passwd")
    assert ei.value.code == "forbidden_path"


@pytest.mark.asyncio
async def test_read_file_size_and_not_a_file_map_to_specific_exceptions() -> None:
    transport = FakeTransport(
        [
            _ok(
                {
                    "token": "2" * 64,
                    "granted_scopes": ["fs:read"],
                    "granted_fs_roots": ["/tmp"],
                    "expires_in": 3600,
                }
            ),
            _err(400, "path_too_large"),
            _err(404, "not_a_file"),
            _err(404, "path_not_found"),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    await client.handshake(scopes=["fs:read"], fs_roots=["/tmp"])
    with pytest.raises(xlpod.PathTooLarge):
        await client.read_file("/tmp/big.bin")
    with pytest.raises(xlpod.NotAFile):
        await client.read_file("/tmp/dir")
    with pytest.raises(xlpod.PathNotFound):
        await client.read_file("/tmp/missing")


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
