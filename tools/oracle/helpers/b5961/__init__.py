from collections.abc import Callable
from typing import Final

from pydantic import JsonValue, TypeAdapter

from .embedding import (
    cohere_map,
    cohere_reply,
    cohere_request,
    headers,
    hf_reply,
    hf_request,
    model_config,
)
from .stream import decode_stream
from .types import Function

FUNCTION: Final = TypeAdapter(Function)
RUNNERS: Final[dict[Function, Callable[[dict[str, JsonValue]], JsonValue]]] = {
    "model_config": model_config,
    "headers": headers,
    "hf_request": hf_request,
    "hf_reply": hf_reply,
    "cohere_map": cohere_map,
    "cohere_request": cohere_request,
    "cohere_reply": cohere_reply,
    "decode_stream": decode_stream,
}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    function = FUNCTION.validate_python(input_value["function"])
    return RUNNERS[function](input_value)
