from pydantic import JsonValue

from .values import (
    chat_map_value,
    chat_messages_value,
    chat_supported_value,
    embedding_headers_value,
    embedding_request_value,
    embedding_response_value,
    embedding_url_value,
)
from .views import INPUT, Input


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    parsed: Input = INPUT.validate_python(input_value)
    result: JsonValue
    if parsed["function"] == "chat_supported":
        result = [chat_supported_value(call) for call in parsed["calls"]]
    elif parsed["function"] == "chat_map":
        result = [chat_map_value(call) for call in parsed["calls"]]
    elif parsed["function"] == "chat_messages":
        result = [chat_messages_value(call) for call in parsed["calls"]]
    elif parsed["function"] == "embedding_headers":
        result = [embedding_headers_value(call) for call in parsed["calls"]]
    elif parsed["function"] == "embedding_url":
        result = [embedding_url_value(call) for call in parsed["calls"]]
    elif parsed["function"] == "embedding_request":
        result = [embedding_request_value(call) for call in parsed["calls"]]
    else:
        result = [embedding_response_value(call) for call in parsed["calls"]]
    return result
