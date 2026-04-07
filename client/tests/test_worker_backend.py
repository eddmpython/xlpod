"""Phase 7 — Python-level unit tests for the worker's Excel backend
selector.

The worker script lives at
``launcher/xlpod-server/src/worker/python_worker.py``; it is loaded by
the Rust launcher via ``include_str!`` in production. We import it
here as a regular Python module via ``importlib.util`` so we can
exercise the backend selection helper without spinning up the worker
process or going through HTTP.

These tests do **not** require Excel to be running. They assert that
``_select_backend`` honours the ``XLPOD_WORKER_BACKEND`` env var:

  - ``xlwings`` forces the xlwings backend; if unavailable, the
    response is a structured error_code (never the pywin32 backend).
  - ``pywin32`` forces the pywin32 backend; same fallback rule.
  - ``auto`` (or unset) tries xlwings first then pywin32, and the
    backend that wins must be one of those two names — never some
    third option.

We accept both the live-backend case (object) and the unavailable
case (dict with error_code) so the test passes on:
  - dev box with Excel running       → live backend
  - dev box with Excel closed        → ``excel_not_running``
  - CI box with no pywin32/xlwings   → ``excel_not_available``
"""

# This test imports a script outside the client/ package; the path
# manipulation is intentional and isolated to this file.
# ruff: noqa: E402

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
from typing import Any
from unittest import mock

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
WORKER_SCRIPT = (
    REPO_ROOT
    / "launcher"
    / "xlpod-server"
    / "src"
    / "worker"
    / "python_worker.py"
)


def _load_worker_module() -> Any:
    spec = importlib.util.spec_from_file_location(
        "xlpod_worker_under_test", WORKER_SCRIPT
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.fixture(scope="module")
def worker_mod() -> Any:
    if not WORKER_SCRIPT.exists():
        pytest.skip(f"worker script not found at {WORKER_SCRIPT}")
    return _load_worker_module()


def _assert_unavailable_or_named(result: Any, allowed_names: set[str]) -> None:
    if isinstance(result, dict):
        assert result.get("ok") is False
        assert result.get("error_code") in (
            "excel_not_available",
            "excel_not_running",
        )
    else:
        assert getattr(result, "name", None) in allowed_names


def test_select_backend_xlwings_forced_never_returns_pywin32(worker_mod: Any) -> None:
    with mock.patch.dict(os.environ, {"XLPOD_WORKER_BACKEND": "xlwings"}):
        result = worker_mod._select_backend()
    _assert_unavailable_or_named(result, allowed_names={"xlwings"})


def test_select_backend_pywin32_forced_never_returns_xlwings(worker_mod: Any) -> None:
    with mock.patch.dict(os.environ, {"XLPOD_WORKER_BACKEND": "pywin32"}):
        result = worker_mod._select_backend()
    _assert_unavailable_or_named(result, allowed_names={"pywin32"})


def test_select_backend_auto_returns_one_of_two_known_backends(worker_mod: Any) -> None:
    env = dict(os.environ)
    env.pop("XLPOD_WORKER_BACKEND", None)
    with mock.patch.dict(os.environ, env, clear=True):
        result = worker_mod._select_backend()
    _assert_unavailable_or_named(result, allowed_names={"xlwings", "pywin32"})


def test_select_backend_unknown_value_falls_through_to_auto(worker_mod: Any) -> None:
    with mock.patch.dict(os.environ, {"XLPOD_WORKER_BACKEND": "garbage"}):
        result = worker_mod._select_backend()
    # Garbage is not "xlwings" or "pywin32", so the worker treats it as
    # "auto" and tries both — the result must still be one of the two
    # known backends or a structured unavailable response.
    _assert_unavailable_or_named(result, allowed_names={"xlwings", "pywin32"})
