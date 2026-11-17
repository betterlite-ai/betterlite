from collections.abc import Callable
from typing import Final

from litellm.llms.anthropic.chat.handler import AnthropicChatCompletion
from pydantic import JsonValue

from .answers import pre_call_value, reply_answer, request_value, stream_answer
from .wire import Attempt, Input, JsonObject


def _request_of(input_value: Input, attempt: Attempt) -> JsonValue:
    if not attempt.requests:
        return None
    names = input_value["request_header_names"]
    return request_value(attempt.requests[0], names)


def _messages_post_request(input_value: Input) -> JsonValue:
    asynchronous = Attempt(input_value, asynchronous=True)
    synchronous = Attempt(input_value, asynchronous=False)
    answer: JsonObject = {
        "async": {
            "pre_call": pre_call_value(asynchronous.recorder),
            "request": _request_of(input_value, asynchronous),
        },
        "sync": {
            "pre_call": pre_call_value(synchronous.recorder),
            "request": _request_of(input_value, synchronous),
        },
    }
    return answer


def _buffered_success(input_value: Input) -> JsonValue:
    asynchronous = Attempt(input_value, asynchronous=True)
    answer: JsonObject = {"async": reply_answer(asynchronous)}
    answer["sync"] = reply_answer(Attempt(input_value, asynchronous=False))
    return answer


def _stream_request(input_value: Input) -> JsonValue:
    names = input_value["request_header_names"]
    asynchronous = Attempt(input_value, asynchronous=True)
    synchronous = Attempt(input_value, asynchronous=False)
    answer: JsonObject = {"async": stream_answer(asynchronous, names)}
    answer["sync"] = stream_answer(synchronous, names)
    return answer


def _pre_stream_error(input_value: Input) -> JsonValue:
    Attempt(input_value, asynchronous=input_value["acompletion"])
    raise ValueError("the call was expected to fail before the transport and did not")


def _native_error_body(input_value: Input) -> JsonValue:
    Attempt(input_value, asynchronous=input_value["acompletion"])
    raise ValueError("the call was expected to project the native error body and did not")


def _chat_failure_projection(input_value: Input) -> JsonValue:
    Attempt(input_value, asynchronous=True)
    raise ValueError("the call was expected to fail and did not")


def _unsupported_embedding(_input_value: Input) -> JsonValue:
    AnthropicChatCompletion().embedding()
    answer: JsonObject = {"reply": None, "posted": 0}
    return answer


SCENARIOS: Final[dict[str, Callable[[Input], JsonValue]]] = {
    "messages_post_request": _messages_post_request,
    "buffered_success": _buffered_success,
    "stream_request": _stream_request,
    "pre_stream_error": _pre_stream_error,
    "native_error_body": _native_error_body,
    "unsupported_embedding": _unsupported_embedding,
    "chat_failure_projection": _chat_failure_projection,
}


def run(input_value: Input) -> JsonValue:
    return SCENARIOS[input_value["scenario"]](input_value)
