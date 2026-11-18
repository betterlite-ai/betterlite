from collections.abc import Callable
from typing import Final

import litellm
from pydantic import JsonValue

from .answers import JSON, pre_call_value, reply_answer, request_value, stream_answer
from .wire import EMBEDDING_HANDLER, Attempt, Input, JsonObject

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True


def _attempt_value(input_value: Input, attempt: Attempt) -> JsonObject:
    names = input_value["request_header_names"]
    request: JsonValue = None
    if attempt.requests:
        request = request_value(attempt.requests[0], names)
    return {"pre_call": pre_call_value(attempt.recorder), "request": request}


def _messages_post_request(input_value: Input) -> JsonValue:
    asynchronous = Attempt(input_value, asynchronous=True)
    synchronous = Attempt(input_value, asynchronous=False)
    answer: JsonObject = {"async": _attempt_value(input_value, asynchronous)}
    answer["sync"] = _attempt_value(input_value, synchronous)
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
    answer: JsonObject = {"reply": EMBEDDING_HANDLER.embedding()}
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
    scenario = SCENARIOS[input_value["scenario"]]
    return JSON.validate_python(scenario(input_value))
