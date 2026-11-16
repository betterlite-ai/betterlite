from typing import TypeAlias

import litellm
from pydantic import JsonValue

from .agents import RegistryInput, registry
from .requests import EmptyInput, RequestInput, empty_request, request
from .responses import ResponseInput, response
from .streams import StreamInput, stream_chunks

__all__ = ["litellm", "run"]

Input: TypeAlias = RegistryInput | RequestInput | EmptyInput | ResponseInput | StreamInput


def run(input_value: Input) -> JsonValue:
    if input_value["op"] == "registry":
        return registry(input_value)
    if input_value["op"] == "request":
        return request(input_value)
    if input_value["op"] == "request_empty":
        return empty_request()
    if input_value["op"] == "response":
        return response(input_value)
    return stream_chunks(input_value)
