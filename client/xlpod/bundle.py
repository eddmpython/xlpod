"""Read and write the xlpod bundle inside an `.xlsx` file.

Phase 10 ships a pure-stdlib reader/writer that operates on the zip
container directly via :mod:`zipfile`. No openpyxl, no pandas, no
lxml — keeping the dependency surface zero is what lets the same
module run inside Pyodide.

The bundle schema lives at ``proto/xlpod.bundle.md`` (the SSOT).
This module is the *Python* implementation of that schema; the
launcher's worker reuses it via ``import xlpod.bundle`` so the
on-disk shape stays identical between the two sides.

Hard caps:

- Total bundle JSON: 64 MiB after encoding.
- Single Pyodide snapshot: same 64 MiB cap, base64+zlib encoded.

The bundle is stored as a single Office Open XML custom part at
``customXml/xlpodBundle.json`` with content type
``application/vnd.xlpod.bundle+json``. ``[Content_Types].xml`` is
patched to register the part. Existing custom parts (e.g. the
xlwings Lite Python source part) are preserved on every write.
"""

from __future__ import annotations

import base64
import io
import json
import zipfile
import zlib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional

from .errors import XlpodError

# ---------------------------------------------------------------------------
# Constants — kept in sync with proto/xlpod.bundle.md
# ---------------------------------------------------------------------------

BUNDLE_NAMESPACE = "urn:xlpod:bundle:v1"
BUNDLE_PART_PATH = "customXml/xlpodBundle.json"
BUNDLE_CONTENT_TYPE = "application/vnd.xlpod.bundle+json"
BUNDLE_SCHEMA_VERSION = 1
MAX_BUNDLE_BYTES = 64 * 1024 * 1024  # 64 MiB hard cap
CONTENT_TYPES_PATH = "[Content_Types].xml"


# ---------------------------------------------------------------------------
# Public exceptions
# ---------------------------------------------------------------------------


class BundleError(XlpodError):
    """Base for every bundle reader/writer error."""


class BundleNotFound(BundleError):
    """The .xlsx file is valid OOXML but does not contain an xlpod bundle."""


class BundleTooLarge(BundleError):
    """Bundle payload exceeds :data:`MAX_BUNDLE_BYTES`."""


class BundleSchemaMismatch(BundleError):
    """Bundle's schema version is newer than this client knows about."""


class BundleCorrupt(BundleError):
    """Bundle JSON is malformed or the zip itself is broken."""


# ---------------------------------------------------------------------------
# Data shape
# ---------------------------------------------------------------------------


@dataclass
class BundlePayload:
    """In-memory representation of a parsed xlpod bundle."""

    schema_version: int = BUNDLE_SCHEMA_VERSION
    created_ms: int = 0
    workbook_fingerprint: Optional[str] = None
    launcher_min_version: Optional[str] = None
    pyodide_snapshot: Optional[bytes] = None
    pyodide_encoding: Optional[str] = None
    ai_sessions: List[Dict[str, Any]] = field(default_factory=list)
    python_modules: List[str] = field(default_factory=list)

    def to_dict(self) -> Dict[str, Any]:
        out: Dict[str, Any] = {
            "schema": BUNDLE_NAMESPACE,
            "metadata": {
                "created_ms": self.created_ms,
                "schema_version": self.schema_version,
            },
            "ai_history": {"sessions": self.ai_sessions},
            "python_modules": self.python_modules,
        }
        if self.launcher_min_version:
            out["metadata"]["launcher_min_version"] = self.launcher_min_version
        if self.workbook_fingerprint:
            out["metadata"]["workbook_fingerprint"] = self.workbook_fingerprint
        if self.pyodide_snapshot is not None:
            out["pyodide"] = {
                "encoding": self.pyodide_encoding or "base64+zlib",
                "snapshot": _encode_snapshot(self.pyodide_snapshot),
            }
        return out

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "BundlePayload":
        if not isinstance(data, dict):
            raise BundleCorrupt("bundle root is not an object")
        schema = data.get("schema")
        if schema != BUNDLE_NAMESPACE:
            raise BundleSchemaMismatch(
                f"unknown bundle schema: {schema!r} (this client expects {BUNDLE_NAMESPACE!r})"
            )
        metadata = data.get("metadata") or {}
        version = int(metadata.get("schema_version", 0) or 0)
        if version > BUNDLE_SCHEMA_VERSION:
            raise BundleSchemaMismatch(
                f"bundle schema_version={version} is newer than this client ({BUNDLE_SCHEMA_VERSION})"
            )
        pyodide = data.get("pyodide") or {}
        snapshot_str = pyodide.get("snapshot")
        snapshot_bytes: Optional[bytes] = None
        if isinstance(snapshot_str, str) and snapshot_str:
            snapshot_bytes = _decode_snapshot(snapshot_str)
        ai_history = data.get("ai_history") or {}
        sessions = ai_history.get("sessions") or []
        if not isinstance(sessions, list):
            raise BundleCorrupt("ai_history.sessions is not an array")
        modules = data.get("python_modules") or []
        if not isinstance(modules, list):
            raise BundleCorrupt("python_modules is not an array")
        return cls(
            schema_version=version,
            created_ms=int(metadata.get("created_ms", 0) or 0),
            workbook_fingerprint=metadata.get("workbook_fingerprint"),
            launcher_min_version=metadata.get("launcher_min_version"),
            pyodide_snapshot=snapshot_bytes,
            pyodide_encoding=pyodide.get("encoding"),
            ai_sessions=list(sessions),
            python_modules=list(modules),
        )


# ---------------------------------------------------------------------------
# Snapshot encoding helpers
# ---------------------------------------------------------------------------


def _encode_snapshot(blob: bytes) -> str:
    """Encode raw snapshot bytes for the JSON wire.

    Phase 10 ships zlib + base64 (stdlib only). The Phase 9 plan
    mentioned zstd; if a future version installs the ``zstandard``
    extra we can swap by changing the encoding tag and the decode
    side to sniff.
    """
    if len(blob) > MAX_BUNDLE_BYTES:
        raise BundleTooLarge(
            f"snapshot is {len(blob)} bytes; cap is {MAX_BUNDLE_BYTES}"
        )
    compressed = zlib.compress(blob, level=6)
    return base64.b64encode(compressed).decode("ascii")


def _decode_snapshot(encoded: str) -> bytes:
    try:
        compressed = base64.b64decode(encoded)
    except Exception as exc:
        raise BundleCorrupt(f"snapshot base64 decode failed: {exc}") from exc
    try:
        return zlib.decompress(compressed)
    except zlib.error as exc:
        raise BundleCorrupt(f"snapshot zlib decompress failed: {exc}") from exc


# ---------------------------------------------------------------------------
# Reader / writer
# ---------------------------------------------------------------------------


def _wrap_xml_envelope(json_bytes: bytes) -> bytes:
    return (
        b'<?xml version="1.0" encoding="UTF-8"?>\n'
        b'<xlpodBundle xmlns="' + BUNDLE_NAMESPACE.encode("ascii") + b'">\n'
        b"  <body><![CDATA[" + json_bytes + b"]]></body>\n"
        b"</xlpodBundle>\n"
    )


def _unwrap_xml_envelope(xml_bytes: bytes) -> bytes:
    """Pull the JSON body out of the tiny XML envelope. Tolerant of
    minor whitespace differences."""
    cdata_open = b"<![CDATA["
    cdata_close = b"]]>"
    start = xml_bytes.find(cdata_open)
    end = xml_bytes.find(cdata_close)
    if start < 0 or end < 0 or end < start:
        raise BundleCorrupt("bundle envelope is missing CDATA section")
    return xml_bytes[start + len(cdata_open) : end]


class BundleReader:
    """Read the xlpod bundle (if any) from an `.xlsx` file."""

    def __init__(self, path: str | Path) -> None:
        self._path = Path(path)
        if not self._path.exists():
            raise FileNotFoundError(self._path)

    def read(self) -> BundlePayload:
        try:
            with zipfile.ZipFile(self._path, "r") as zf:
                if BUNDLE_PART_PATH not in zf.namelist():
                    raise BundleNotFound(
                        f"no xlpod bundle in {self._path.name}; expected {BUNDLE_PART_PATH}"
                    )
                raw = zf.read(BUNDLE_PART_PATH)
        except zipfile.BadZipFile as exc:
            raise BundleCorrupt(f"not a valid xlsx zip: {exc}") from exc
        if len(raw) > MAX_BUNDLE_BYTES:
            raise BundleTooLarge(
                f"bundle part is {len(raw)} bytes; cap is {MAX_BUNDLE_BYTES}"
            )
        body = _unwrap_xml_envelope(raw)
        try:
            data = json.loads(body.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise BundleCorrupt(f"bundle JSON parse failed: {exc}") from exc
        return BundlePayload.from_dict(data)

    def has_bundle(self) -> bool:
        try:
            with zipfile.ZipFile(self._path, "r") as zf:
                return BUNDLE_PART_PATH in zf.namelist()
        except zipfile.BadZipFile:
            return False


class BundleWriter:
    """Write or replace the xlpod bundle inside an `.xlsx` file.

    Operates by streaming every part of the existing zip into a new
    in-memory zip, replacing or appending the bundle part, and
    finally renaming the new file over the old one. The Lite custom
    parts (and any other unknown parts) are preserved byte-for-byte.
    """

    def __init__(self, path: str | Path) -> None:
        self._path = Path(path)
        if not self._path.exists():
            raise FileNotFoundError(self._path)

    def write(self, payload: BundlePayload) -> None:
        json_bytes = json.dumps(payload.to_dict()).encode("utf-8")
        wrapped = _wrap_xml_envelope(json_bytes)
        if len(wrapped) > MAX_BUNDLE_BYTES:
            raise BundleTooLarge(
                f"encoded bundle is {len(wrapped)} bytes; cap is {MAX_BUNDLE_BYTES}"
            )

        try:
            with zipfile.ZipFile(self._path, "r") as zin:
                items: list[tuple[zipfile.ZipInfo, bytes]] = []
                replaced_bundle = False
                for info in zin.infolist():
                    data = zin.read(info.filename)
                    if info.filename == BUNDLE_PART_PATH:
                        items.append((info, wrapped))
                        replaced_bundle = True
                    elif info.filename == CONTENT_TYPES_PATH:
                        items.append((info, _patch_content_types(data)))
                    else:
                        items.append((info, data))
                if not replaced_bundle:
                    new_info = zipfile.ZipInfo(BUNDLE_PART_PATH)
                    new_info.compress_type = zipfile.ZIP_DEFLATED
                    items.append((new_info, wrapped))
        except zipfile.BadZipFile as exc:
            raise BundleCorrupt(f"not a valid xlsx zip: {exc}") from exc

        # Atomic replace via temp file in same directory.
        tmp_path = self._path.with_suffix(self._path.suffix + ".xlpod.tmp")
        with zipfile.ZipFile(tmp_path, "w", zipfile.ZIP_DEFLATED) as zout:
            for info, data in items:
                zout.writestr(info, data)
        tmp_path.replace(self._path)


def _patch_content_types(data: bytes) -> bytes:
    """Ensure ``[Content_Types].xml`` declares the bundle part type.

    The patch is conservative — if the override is already present
    we leave the file alone, otherwise we splice a new ``Override``
    just before the closing ``</Types>`` tag.
    """
    text = data.decode("utf-8")
    needle = (
        f'<Override PartName="/{BUNDLE_PART_PATH}" '
        f'ContentType="{BUNDLE_CONTENT_TYPE}"/>'
    )
    if needle in text:
        return data
    if "</Types>" not in text:
        return data
    patched = text.replace("</Types>", f"  {needle}\n</Types>")
    return patched.encode("utf-8")
