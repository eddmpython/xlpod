"""Phase 12 — headless smoke test of the Lite panel script.

The panel itself lives at ``examples/living_workbook/lite_panel.py``;
in production it runs inside xlwings Lite's Pyodide. We import it
here via ``importlib.util`` so the test process can poke at the
``LivingWorkbookPanel`` class with a stub transport instead of a
real launcher.

This is a *smoke* test: the goals are
  - the file imports without crashing on a stock CPython
  - ``on_open`` completes the handshake + open_session round trip
  - ``send`` round-trips a fake assistant reply through the
    fake transport
  - ``save`` is a no-op when no workbook path is set (panel
    fall-through, no exception)
"""

# ruff: noqa: E402

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
from typing import Any, List, Mapping, Optional

import pytest

import xlpod
from xlpod._transport import TransportResponse


REPO_ROOT = Path(__file__).resolve().parents[2]
LITE_PANEL = REPO_ROOT / "examples" / "living_workbook" / "lite_panel.py"


def _load_panel_module() -> Any:
    spec = importlib.util.spec_from_file_location(
        "xlpod_lite_panel_under_test", LITE_PANEL
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.fixture(scope="module")
def panel_mod() -> Any:
    if not LITE_PANEL.exists():
        pytest.skip(f"lite_panel.py not found at {LITE_PANEL}")
    return _load_panel_module()


# ---------------------------------------------------------------------------
# Tiny FakeTransport that returns scripted responses without a launcher.
# (Same shape as the one in test_client.py — duplicated here so the file
# stays self-contained and easy to read.)
# ---------------------------------------------------------------------------


class FakeTransport:
    def __init__(self, responses: List[TransportResponse]) -> None:
        self._responses = list(responses)
        self.recorded: List[dict] = []
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
            {"method": method, "url": url, "json_body": json_body}
        )
        if not self._responses:
            raise AssertionError("FakeTransport ran out of canned responses")
        return self._responses.pop(0)

    async def aclose(self) -> None:
        self.closed = True


def _ok(payload: dict) -> TransportResponse:
    return TransportResponse(status_code=200, body=json.dumps(payload).encode())


def _err(status: int, code: str) -> TransportResponse:
    return TransportResponse(
        status_code=status,
        body=json.dumps({"code": code, "message": "x"}).encode(),
    )


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_panel_on_open_handshake_and_session(panel_mod: Any) -> None:
    transport = FakeTransport(
        [
            _ok(
                {
                    "token": "p" * 64,
                    "granted_scopes": ["ai:provider:call", "run:python"],
                    "expires_in": 3600,
                }
            ),
            _ok(
                {
                    "session_id": "12345678-1234-1234-1234-123456789012",
                    "provider": "anthropic",
                    "model": "claude-opus-4-6",
                    "granted_scopes": ["ai:provider:call"],
                    "opened_ms": 1,
                }
            ),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    events: list[tuple[str, Any]] = []
    panel = panel_mod.LivingWorkbookPanel(
        client, on_event=lambda kind, payload: events.append((kind, payload))
    )
    await panel.on_open(workbook_path=None)
    assert any(k == "session_opened" for k, _ in events)


@pytest.mark.asyncio
async def test_panel_send_round_trips_assistant_reply(panel_mod: Any) -> None:
    transport = FakeTransport(
        [
            _ok(
                {
                    "token": "q" * 64,
                    "granted_scopes": ["ai:provider:call"],
                    "expires_in": 3600,
                }
            ),
            _ok(
                {
                    "session_id": "abcdef00-0000-0000-0000-000000000000",
                    "provider": "anthropic",
                    "model": "claude-opus-4-6",
                    "granted_scopes": ["ai:provider:call"],
                    "opened_ms": 1,
                }
            ),
            _ok(
                {
                    "session_id": "abcdef00-0000-0000-0000-000000000000",
                    "message": {
                        "role": "assistant",
                        "content": [{"type": "text", "text": "hi back"}],
                    },
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 1, "output_tokens": 2},
                }
            ),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    panel = panel_mod.LivingWorkbookPanel(client)
    await panel.on_open(workbook_path=None)
    result = await panel.send("hello")
    assert result["ok"] is True
    assert result["message"]["content"][0]["text"] == "hi back"


@pytest.mark.asyncio
async def test_panel_send_handles_provider_unconfigured(panel_mod: Any) -> None:
    transport = FakeTransport(
        [
            _ok(
                {
                    "token": "r" * 64,
                    "granted_scopes": ["ai:provider:call"],
                    "expires_in": 3600,
                }
            ),
            _ok(
                {
                    "session_id": "00000000-0000-0000-0000-000000000001",
                    "provider": "anthropic",
                    "model": "claude-opus-4-6",
                    "granted_scopes": ["ai:provider:call"],
                    "opened_ms": 1,
                }
            ),
            _err(412, "ai_provider_unconfigured"),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    events: list[tuple[str, Any]] = []
    panel = panel_mod.LivingWorkbookPanel(
        client, on_event=lambda kind, payload: events.append((kind, payload))
    )
    await panel.on_open(workbook_path=None)
    result = await panel.send("hello")
    assert result["ok"] is False
    assert any(k == "provider_missing" for k, _ in events)


@pytest.mark.asyncio
async def test_panel_save_skipped_without_workbook_path(panel_mod: Any) -> None:
    transport = FakeTransport(
        [
            _ok(
                {
                    "token": "s" * 64,
                    "granted_scopes": ["ai:provider:call"],
                    "expires_in": 3600,
                }
            ),
            _ok(
                {
                    "session_id": "00000000-0000-0000-0000-000000000002",
                    "provider": "anthropic",
                    "model": "claude-opus-4-6",
                    "granted_scopes": ["ai:provider:call"],
                    "opened_ms": 1,
                }
            ),
        ]
    )
    client = xlpod.AsyncClient(transport=transport)
    events: list[tuple[str, Any]] = []
    panel = panel_mod.LivingWorkbookPanel(
        client, on_event=lambda kind, payload: events.append((kind, payload))
    )
    await panel.on_open(workbook_path=None)
    await panel.save()
    assert any(k == "save_skipped" for k, _ in events)
