from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

from litellm.llms.anthropic.pass_through.messages.streaming_iterator import (
    _anthropic_event_payload,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/streaming_iterator.py#L94
    _incomplete_stream_error_sse_event,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/streaming_iterator.py#L193
    _is_message_stop_chunk,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/streaming_iterator.py#L33
    _is_provider_error_chunk,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/streaming_iterator.py#L136
    _is_terminal_stream_chunk,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/streaming_iterator.py#L163
    anthropic_messages_response_as_sse_events,
    is_anthropic_content_delta_chunk,
    is_anthropic_ping_chunk,
    parse_anthropic_error_event,
    parse_anthropic_refusal_stop_details,
)
from pydantic import JsonValue as Json

Chunk: TypeAlias = bytes | Json

Action: TypeAlias = Literal[
    "raw_chunk_classifiers",
    "data_line_extraction",
    "native_error_and_refusal_projection",
    "response_as_sse_events",
    "incomplete_error_literal",
]


class Input(TypedDict):
    action: Action
    raw_bytes: NotRequired[list[int]]
    value: NotRequired[Json]
    event: NotRequired[str]
    response: NotRequired[Json]


def chunk_of(input_value: Input) -> Chunk:
    encoded = input_value.get("raw_bytes")
    if encoded is None:
        return input_value.get("value")
    return bytes(encoded)


def classify(chunk: Chunk) -> Json:
    result: Json = {
        "message_stop": _is_message_stop_chunk(chunk),
        "ping": is_anthropic_ping_chunk(chunk),
        "content_delta": is_anthropic_content_delta_chunk(chunk),
        "provider_error": _is_provider_error_chunk(chunk),
        "terminal": _is_terminal_stream_chunk(chunk),
    }
    return result


def event_payload(chunk: Chunk, event: str) -> Json:
    payload = _anthropic_event_payload(chunk, event)
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/streaming_iterator.py#L94
        Json, payload
    )


def error_projection(chunk: Chunk) -> Json:
    parsed = parse_anthropic_error_event(chunk)
    error: Json = (
        None
        if parsed is None
        else {"type": parsed[0], "message": parsed[1], "status": parsed[2]}
    )
    details = parse_anthropic_refusal_stop_details(chunk)
    result: Json = {
        "terminal": _is_terminal_stream_chunk(chunk),
        "error": error,
        "refusal_stop_details": cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/streaming_iterator.py#L114
            Json, details
        ),
    }
    return result


def response_frames(response: Json) -> Json:
    frames = anthropic_messages_response_as_sse_events(
        response  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/streaming_iterator.py#L251
    )
    return [frame.decode() for frame in frames]


def incomplete_literal() -> Json:
    frame = _incomplete_stream_error_sse_event()
    return frame.decode()


def run(input_value: Input) -> Json:
    action = input_value["action"]
    if action == "incomplete_error_literal":
        return incomplete_literal()
    if action == "response_as_sse_events":
        response = input_value.get("response")
        if response is None:
            raise ValueError("the response field is required")
        return response_frames(response)
    chunk = chunk_of(input_value)
    if action == "raw_chunk_classifiers":
        return classify(chunk)
    if action == "data_line_extraction":
        event = input_value.get("event")
        if event is None:
            raise ValueError("the event field is required")
        return event_payload(chunk, event)
    if action == "native_error_and_refusal_projection":
        return error_projection(chunk)
    raise ValueError(f"unknown messages_sse_events action: {action}")
