import os
from typing import Protocol, TypeAlias, cast

# LiteLLM reads LITELLM_RUST per call; false pins the Python fallback the oracle compares against.
os.environ["LITELLM_RUST"] = "false"

import litellm
import tiktoken
from litellm.rust_bridge import tokenizer as tokenizer_module
from tokenizers import Tokenizer

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_PYTHON_ENCODING = "_python_encoding"


class _EncodingLookup(Protocol):
    def __call__(self, name: str) -> tiktoken.Encoding: ...


_python_encoding = cast(_EncodingLookup, getattr(tokenizer_module, _PYTHON_ENCODING))


def _text(fields: dict[str, Json]) -> str:
    value = fields.get("text")
    if not isinstance(value, str):
        raise ValueError("text must be a string")
    return value


def _tokens(fields: dict[str, Json]) -> list[int]:
    value = fields.get("tokens")
    if not isinstance(value, list):
        raise ValueError("tokens must be a list")
    tokens: list[int] = []
    for item in value:
        if not isinstance(item, int) or isinstance(item, bool):
            raise ValueError("tokens must be integers")
        tokens.append(item)
    return tokens


def _engine(fields: dict[str, Json]) -> tiktoken.Encoding | Tokenizer:
    name = fields.get("engine")
    match name:
        case "cl100k_base" | "o200k_base" | "p50k_base" | "p50k_edit" | "o200k_harmony":
            return _python_encoding(name)
        case "anthropic":
            return Tokenizer.from_str(litellm.utils.claude_json_str)
        case "custom":
            document = fields.get("tokenizer_json")
            if not isinstance(document, str):
                raise ValueError("tokenizer_json must be a string")
            return Tokenizer.from_str(document)
        case other:
            raise ValueError(f"unknown engine: {other!r}")


def run(input_value: dict[str, Json]) -> Json:
    engine = _engine(input_value)
    match input_value.get("operation"):
        case "encode":
            source = _text(input_value)
            if isinstance(engine, tiktoken.Encoding):
                return cast(Json, engine.encode(source, disallowed_special=()))
            return cast(Json, engine.encode(source).ids)
        case "count":
            source = _text(input_value)
            if isinstance(engine, tiktoken.Encoding):
                return len(engine.encode(source, disallowed_special=()))
            return len(engine.encode(source).ids)
        case "decode":
            ids = _tokens(input_value)
            if isinstance(engine, tiktoken.Encoding):
                return engine.decode(ids)
            return engine.decode(ids, skip_special_tokens=True)
        case other:
            raise ValueError(f"unknown operation: {other!r}")
