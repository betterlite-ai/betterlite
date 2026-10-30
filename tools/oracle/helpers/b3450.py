import base64
import re
import sys
import types
from collections.abc import Callable
from io import BytesIO
from typing import TypeAlias, cast

from litellm.rag.ingestion.file_parsers.pdf_parser import extract_text_from_pdf
from litellm.rag.main import get_ingestion_class
from litellm.rag.text_splitters.recursive_character_text_splitter import (
    RecursiveCharacterTextSplitter,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_TEXT_SHOW_OPERATOR = re.compile(rb"\(((?:[^()\\]|\\.)*)\)\s*Tj")


class _StubPage:
    def __init__(self, text: str) -> None:
        self._text = text

    def extract_text(self) -> str:
        return self._text


class _StubReader:
    def __init__(self, stream: BytesIO) -> None:
        matches: list[bytes] = _TEXT_SHOW_OPERATOR.findall(stream.read())
        self.pages = [_StubPage(match.decode("latin-1")) for match in matches]


def _install_stub_reader(module_name: str) -> None:
    stub = types.ModuleType(module_name)
    stub.PdfReader = _StubReader  # type: ignore[attr-defined]
    sys.modules[module_name] = stub


def _block_import(module_name: str) -> None:
    sys.modules[module_name] = None  # type: ignore[assignment]


def _split(input_value: dict[str, Json]) -> Json:
    chunk_size = cast(int, input_value["chunk_size"])
    chunk_overlap = cast(int, input_value["chunk_overlap"])
    separators = cast(list[str] | None, input_value.get("separators"))
    text = cast(str, input_value["text"])
    splitter = RecursiveCharacterTextSplitter(
        chunk_size=chunk_size, chunk_overlap=chunk_overlap, separators=separators
    )
    return cast(Json, splitter.split_text(text))


def _extract_pdf(input_value: dict[str, Json]) -> Json:
    pdf_bytes = base64.b64decode(cast(str, input_value["pdf_base64"]))
    block_pypdf = cast(bool, input_value.get("block_pypdf", False))
    block_pypdf2 = cast(bool, input_value.get("block_pypdf2", False))
    saved = {name: sys.modules.get(name) for name in ("pypdf", "PyPDF2")}
    try:
        _block_import("pypdf") if block_pypdf else _install_stub_reader("pypdf")
        _block_import("PyPDF2") if block_pypdf2 else _install_stub_reader("PyPDF2")
        extracted = extract_text_from_pdf(pdf_bytes)
    finally:
        for name, module in saved.items():
            if module is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = module
    return {"text": extracted.strip() if extracted is not None else None}


def _provider_lookup(input_value: dict[str, Json]) -> Json:
    provider = cast(str, input_value["provider"])
    ingestion_class = get_ingestion_class(provider)
    return ingestion_class.__name__


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "split": _split,
    "extract_pdf": _extract_pdf,
    "provider_lookup": _provider_lookup,
}


def run(input_value: dict[str, Json]) -> Json:
    mode = cast(str, input_value["mode"])
    if mode not in HANDLERS:
        raise ValueError(f"unsupported mode {mode}")
    return HANDLERS[mode](input_value)
