import httpx
import litellm
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from pydantic import JsonValue

from .configs import Recorder
from .values import (
    chat_map_value,
    chat_supported_value,
    embedding_headers_value,
    embedding_request_value,
    embedding_response_value,
    embedding_url_value,
)
from .views import INPUT, Input, MessagesCall


def chat_messages_value(call: MessagesCall) -> JsonValue:
    recorder = Recorder()
    litellm.completion(
        model=f"hosted_vllm/{call['model']}",
        messages=call["messages"],
        api_base="http://hosted-vllm.invalid",
        api_key="k",
        client=HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(recorder.handle))),
    )
    return recorder.body["messages"]


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
