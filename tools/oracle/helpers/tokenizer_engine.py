import os
from typing import Protocol, TypeAlias, cast

# LiteLLM reads LITELLM_RUST per call; false pins the Python fallback the oracle compares against.
os.environ["LITELLM_RUST"] = "false"

import litellm
import tiktoken
from litellm.rust_bridge import tokenizer as tokenizer_module
from tokenizers import Tokenizer

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
_NAMES = frozenset({"cl100k_base", "o200k_base", "p50k_base", "p50k_edit", "o200k_harmony"})
class _Lookup(Protocol):
    def __call__(self, name: str) -> tiktoken.Encoding: ...
_python_encoding = cast(_Lookup, tokenizer_module.__dict__["_python_encoding"])
def _text(fields: dict[str, Json]) -> str:
    value = fields.get("text")
    if not isinstance(value, str):
        raise ValueError("text must be a string")
    return value
def _tokens(fields: dict[str, Json]) -> list[int]:
    value = fields.get("tokens")
    if not isinstance(value, list) or not all(
        isinstance(item, int) and not isinstance(item, bool) for item in value
    ):
        raise ValueError("tokens must be integers")
    return [cast(int, item) for item in value]
def run(input_value: dict[str, Json]) -> Json:
    name = input_value.get("engine")
    if isinstance(name, str) and name in _NAMES:
        engine: tiktoken.Encoding | Tokenizer = _python_encoding(name)
    elif name == "anthropic":
        engine = Tokenizer.from_str(litellm.utils.claude_json_str)
    elif name == "custom" and isinstance(input_value.get("tokenizer_json"), str):
        engine = Tokenizer.from_str(cast(str, input_value.get("tokenizer_json")))
    else:
        raise ValueError(f"unknown engine: {name!r}")
    operation = input_value.get("operation")
    if operation == "encode":
        source = _text(input_value)
        if isinstance(engine, tiktoken.Encoding):
            return cast(Json, engine.encode(source, disallowed_special=()))
        return cast(Json, engine.encode(source).ids)
    if operation == "decode":
        ids = _tokens(input_value)
        if isinstance(engine, tiktoken.Encoding):
            return engine.decode(ids)
        return engine.decode(ids, skip_special_tokens=True)
    raise ValueError(f"unknown operation: {operation!r}")
