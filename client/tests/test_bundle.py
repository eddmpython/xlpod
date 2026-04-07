"""Phase 10 — bundle reader/writer tests.

These tests construct minimal valid `.xlsx` zip containers in
memory (we do not ship a real Excel-saved file as a fixture; the
shape we need is the OOXML zip layout, not a fully-rendered
spreadsheet) and round-trip the xlpod bundle through them.

They run on every Python in the CI matrix (3.10 — 3.13) and use
only stdlib `zipfile`, matching the runtime constraints of the
production module.
"""

# ruff: noqa: E402

from __future__ import annotations

import shutil
import zipfile
from pathlib import Path

import pytest

import xlpod
from xlpod.bundle import (
    BUNDLE_CONTENT_TYPE,
    BUNDLE_PART_PATH,
    CONTENT_TYPES_PATH,
    BundlePayload,
    BundleReader,
    BundleWriter,
    _decode_snapshot,
    _encode_snapshot,
)

MINIMAL_CONTENT_TYPES = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">\n'
    '  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>\n'
    '  <Default Extension="xml" ContentType="application/xml"/>\n'
    '</Types>'
)

MINIMAL_RELS = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>'
)

LITE_CUSTOM_PART = b"# xlwings Lite Python source -- must survive bundle write\nprint('hi')\n"
LITE_PART_PATH = "customXml/item1.xml"


def make_minimal_xlsx(tmp_path: Path, *, with_lite_part: bool = False) -> Path:
    """Build the smallest zip that the bundle reader/writer treats as a
    valid OOXML container."""
    target = tmp_path / "book.xlsx"
    with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr(CONTENT_TYPES_PATH, MINIMAL_CONTENT_TYPES)
        zf.writestr("_rels/.rels", MINIMAL_RELS)
        if with_lite_part:
            zf.writestr(LITE_PART_PATH, LITE_CUSTOM_PART)
    return target


def test_round_trip_writes_and_reads_bundle(tmp_path: Path) -> None:
    book = make_minimal_xlsx(tmp_path)
    payload = BundlePayload(
        created_ms=1234567890123,
        ai_sessions=[
            {
                "session_id": "11111111-2222-3333-4444-555555555555",
                "provider": "anthropic",
                "model": "claude-opus-4-6",
                "messages": [
                    {"role": "user", "content": [{"type": "text", "text": "hi"}]}
                ],
            }
        ],
        python_modules=["pandas"],
    )
    BundleWriter(book).write(payload)

    reader = BundleReader(book)
    assert reader.has_bundle()
    parsed = reader.read()
    assert parsed.created_ms == 1234567890123
    assert parsed.python_modules == ["pandas"]
    assert parsed.ai_sessions[0]["provider"] == "anthropic"
    assert parsed.ai_sessions[0]["messages"][0]["content"][0]["text"] == "hi"


def test_round_trip_preserves_lite_custom_part(tmp_path: Path) -> None:
    book = make_minimal_xlsx(tmp_path, with_lite_part=True)
    BundleWriter(book).write(BundlePayload())
    with zipfile.ZipFile(book, "r") as zf:
        assert LITE_PART_PATH in zf.namelist()
        assert zf.read(LITE_PART_PATH) == LITE_CUSTOM_PART
        assert BUNDLE_PART_PATH in zf.namelist()


def test_writer_patches_content_types_only_once(tmp_path: Path) -> None:
    book = make_minimal_xlsx(tmp_path)
    BundleWriter(book).write(BundlePayload())
    BundleWriter(book).write(BundlePayload())  # second write must not duplicate
    with zipfile.ZipFile(book, "r") as zf:
        ct = zf.read(CONTENT_TYPES_PATH).decode("utf-8")
    needle = (
        f'<Override PartName="/{BUNDLE_PART_PATH}" '
        f'ContentType="{BUNDLE_CONTENT_TYPE}"/>'
    )
    assert ct.count(needle) == 1


def test_reader_raises_when_bundle_missing(tmp_path: Path) -> None:
    book = make_minimal_xlsx(tmp_path)
    reader = BundleReader(book)
    assert not reader.has_bundle()
    with pytest.raises(xlpod.BundleNotFound):
        reader.read()


def test_reader_raises_on_corrupt_zip(tmp_path: Path) -> None:
    book = tmp_path / "broken.xlsx"
    book.write_bytes(b"not a zip at all")
    with pytest.raises(xlpod.BundleCorrupt):
        BundleReader(book).read()


def test_snapshot_round_trip_arbitrary_bytes() -> None:
    blob = bytes(range(256)) * 100  # 25_600 bytes
    encoded = _encode_snapshot(blob)
    decoded = _decode_snapshot(encoded)
    assert decoded == blob


def test_snapshot_too_large_raises() -> None:
    too_big = b"\0" * (xlpod.bundle.MAX_BUNDLE_BYTES + 1)
    with pytest.raises(xlpod.BundleTooLarge):
        _encode_snapshot(too_big)


def test_payload_with_snapshot_round_trip(tmp_path: Path) -> None:
    book = make_minimal_xlsx(tmp_path)
    snapshot = b"PYODIDE-SNAPSHOT-FAKE-BYTES" * 10
    payload = BundlePayload(
        pyodide_snapshot=snapshot,
        pyodide_encoding="base64+zlib",
    )
    BundleWriter(book).write(payload)
    parsed = BundleReader(book).read()
    assert parsed.pyodide_snapshot == snapshot
    assert parsed.pyodide_encoding == "base64+zlib"


def test_schema_mismatch_raises(tmp_path: Path) -> None:
    """Inject a bundle with a future schema version and confirm
    the reader refuses it cleanly."""
    book = make_minimal_xlsx(tmp_path)
    BundleWriter(book).write(BundlePayload())
    # Corrupt the schema_version to a far-future value.
    with zipfile.ZipFile(book, "r") as zf:
        items = [(info, zf.read(info.filename)) for info in zf.infolist()]
    new_book = book.with_name("future.xlsx")
    shutil.copy(book, new_book)
    with zipfile.ZipFile(new_book, "w", zipfile.ZIP_DEFLATED) as zout:
        for info, data in items:
            if info.filename == BUNDLE_PART_PATH:
                # rewrite with a bumped schema_version
                tampered = data.replace(b'"schema_version": 1', b'"schema_version": 99')
                zout.writestr(info, tampered)
            else:
                zout.writestr(info, data)
    with pytest.raises(xlpod.BundleSchemaMismatch):
        BundleReader(new_book).read()
