import base64
import importlib
import importlib.abc
import sys
import types
from collections.abc import Callable, Sequence
from importlib.machinery import ModuleSpec
from typing import TypeAlias, cast

from litellm.rag.ingestion.file_parsers.pdf_parser import extract_text_from_pdf
from litellm.rag.main import get_ingestion_class
from litellm.rag.text_splitters.recursive_character_text_splitter import (
    RecursiveCharacterTextSplitter,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

class _BlockedImportFinder(importlib.abc.MetaPathFinder):
    def __init__(self, blocked_name: str) -> None:
        self._blocked_name = blocked_name

    def find_spec(
        self,
        fullname: str,
        _path: Sequence[str] | None,
        _target: types.ModuleType | None = None,
        /,
    ) -> ModuleSpec | None:
        if fullname == self._blocked_name:
            raise ImportError(f"{fullname} is blocked for this oracle case")
        return None


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
    saved_modules = {name: sys.modules.get(name) for name in ("pypdf", "PyPDF2")}
    saved_meta_path = list(sys.meta_path)
    try:
        pypdf = importlib.import_module("pypdf")
        # Warm lazy imports before hiding pypdf for the fallback call.
        if block_pypdf and not block_pypdf2:
            extract_text_from_pdf(pdf_bytes)
        for name, blocked in (("pypdf", block_pypdf), ("PyPDF2", block_pypdf2)):
            sys.modules.pop(name, None)
            if blocked:
                sys.meta_path.insert(0, _BlockedImportFinder(name))
            else:
                sys.modules[name] = pypdf
        extracted = extract_text_from_pdf(pdf_bytes)
    finally:
        sys.meta_path[:] = saved_meta_path
        for name, module in saved_modules.items():
            if module is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = module
    return {"text": extracted}


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
