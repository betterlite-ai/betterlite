import json
from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.llms.anthropic.pass_through.messages.streaming_iterator import (
    is_anthropic_content_delta_chunk,
    is_anthropic_ping_chunk,
    parse_anthropic_error_event,
    parse_anthropic_refusal_stop_details,
)
from litellm.llms.anthropic.pass_through.messages.utils import (
    INCOMPLETE_STREAM_ERROR_MESSAGE as INCOMPLETE_MESSAGE,
)
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

Chunk: TypeAlias = bytes | Json
Object: TypeAlias = dict[str, Json]
Frame: TypeAlias = list[Json]

# LiteLLM returns Mapping[str, object]; this checks those values as JSON instead of casting them
JSON: TypeAdapter[Json] = TypeAdapter(Json)

# json.detect_encoding places UTF-32 null bytes across 4 bytes and UTF-16 null bytes across 2
MIN_UTF32_BYTES = 4
MIN_UTF16_BYTES = 2
BOMS: tuple[tuple[bytes, str], ...] = (
    (b"\xff\xfe\x00\x00", "utf-32-le"),
    (b"\x00\x00\xfe\xff", "utf-32-be"),
    (b"\xff\xfe", "utf-16-le"),
    (b"\xfe\xff", "utf-16-be"),
    (b"\xef\xbb\xbf", "utf-8"),
)

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


def is_blank(value: Json) -> bool:
    match value:
        case None:
            return True
        case bool():
            return not value
        case int() | float():
            return value == 0
        case str() | list() | dict():
            return not value


def is_object(value: Json | None) -> bool:
    match value:
        case dict():
            return True
        case None | bool() | int() | float() | str() | list():
            return False


def given_or(value: Json | None, fallback: Json) -> Json:
    if value is None or is_blank(value):
        return fallback
    return value


def decode_text(raw: bytes) -> str:
    for bom, encoding in BOMS:
        if raw.startswith(bom):
            return raw[len(bom) :].decode(encoding)
    if len(raw) >= MIN_UTF32_BYTES and raw[0] == 0:
        return raw.decode("utf-16-be" if raw[1] else "utf-32-be")
    if len(raw) >= MIN_UTF32_BYTES and raw[1] == 0:
        return raw.decode("utf-16-le" if raw[2] or raw[3] else "utf-32-le")
    if len(raw) == MIN_UTF16_BYTES and raw[0] == 0:
        return raw.decode("utf-16-be")
    if len(raw) == MIN_UTF16_BYTES and raw[1] == 0:
        return raw.decode("utf-16-le")
    return raw.decode("utf-8")


def decode_data_line(line: bytes) -> Json | None:
    try:
        return JSON.validate_json(decode_text(line))
    except ValueError:
        return None


def event_payload(chunk: Chunk, event: str) -> Json:
    match chunk:
        case bytes():
            for line in chunk.splitlines():
                if not line.startswith(b"data:"):
                    continue
                payload = decode_data_line(line[len(b"data:") :].strip())
                if payload is None:
                    continue
                match payload:
                    case dict():
                        if payload.get("type") == event:
                            return payload
                        continue
                    case list() | str() | int() | float():
                        continue
            return None
        case dict():
            return chunk if chunk.get("type") == event else None
        case None | bool() | int() | float() | str() | list():
            return None


def is_message_stop(chunk: Chunk) -> bool:
    match chunk:
        case bytes():
            return any(line == b"event: message_stop" for line in chunk.splitlines())
        case dict():
            return chunk.get("type") == "message_stop"
        case None | bool() | int() | float() | str() | list():
            return False


def is_provider_error(chunk: Chunk) -> bool:
    payload = event_payload(chunk, "error")
    match payload:
        case dict():
            return is_object(payload.get("error"))
        case None | bool() | int() | float() | str() | list():
            return False


def sse_event(event_type: str, payload: Json) -> str:
    data = json.dumps(payload)
    return f"event: {event_type}\ndata: {data}\n\n"


def indexed_event(event_type: str, index: int, **extra: Json) -> str:
    payload: Json = {"type": event_type, "index": index, **extra}
    return sse_event(event_type, payload)


def block_start_and_deltas(block: Object) -> tuple[Object, list[Object]]:
    match block.get("type"):
        case "tool_use":
            tool_start: Object = {
                "id": block.get("id"),
                "name": block.get("name"),
                "input": {},
                "type": "tool_use",
            }
            tool_delta: Object = {
                "partial_json": json.dumps(given_or(block.get("input"), {})),
                "type": "input_json_delta",
            }
            return tool_start, [tool_delta]
        case "thinking":
            return thinking_start_and_deltas(block)
        case "redacted_thinking":
            redacted_start: Object = {"type": "redacted_thinking", "data": block.get("data")}
            return redacted_start, []
        case _:
            text_delta: Object = {"type": "text_delta", "text": given_or(block.get("text"), "")}
            text_start: Object = {"type": "text", "text": ""}
            return text_start, [text_delta]


def thinking_start_and_deltas(block: Object) -> tuple[Object, list[Object]]:
    thinking_deltas: list[Object] = [
        {"thinking": given_or(block.get("thinking"), ""), "type": "thinking_delta"}
    ]
    signature = block.get("signature")
    match signature:
        case str():
            if signature:
                thinking_deltas.append({"signature": signature, "type": "signature_delta"})
        case None | bool() | int() | float() | list() | dict():
            pass
    return {"thinking": "", "signature": "", "type": "thinking"}, thinking_deltas


def content_block_events(index: int, block: Json) -> Frame:
    match block:
        case dict():
            start_block, deltas = block_start_and_deltas(block)
            frames: Frame = [indexed_event("content_block_start", index, content_block=start_block)]
            for delta in deltas:
                frames.append(indexed_event("content_block_delta", index, delta=delta))
            frames.append(indexed_event("content_block_stop", index))
            return frames
        case None | bool() | int() | float() | str() | list():
            raise ValueError("a content block must be an object")


def render_response(response: Json) -> Frame:
    match response:
        case dict():
            message_usage: Object = {"output_tokens": 0}
            usage = given_or(response.get("usage"), {})
            match usage:
                case dict():
                    message_usage = {**usage, "output_tokens": 0}
                case None | bool() | int() | float() | str() | list():
                    pass
            message: Object = {
                **response, "content": [], "stop_reason": None,
                "stop_sequence": None, "usage": message_usage,
            }
            start_payload: Json = {"type": "message_start", "message": message}
            frames: Frame = [sse_event("message_start", start_payload)]
            blocks = given_or(response.get("content"), [])
            match blocks:
                case list():
                    for index, block in enumerate(blocks):
                        frames.extend(content_block_events(index, block))
                case None | bool() | int() | float() | str() | dict():
                    pass
            delta: Object = {
                "stop_reason": response.get("stop_reason"),
                "stop_sequence": response.get("stop_sequence"),
            }
            delta_payload: Json = {"type": "message_delta", "delta": delta, "usage": usage}
            frames.append(sse_event("message_delta", delta_payload))
            frames.append(sse_event("message_stop", {"type": "message_stop"}))
            return frames
        case None | bool() | int() | float() | str() | list():
            raise ValueError("the response must be an object")


def incomplete_literal() -> str:
    payload: Json = {
        "type": "error",
        "error": {"type": "api_error", "message": INCOMPLETE_MESSAGE},
    }
    return sse_event("error", payload)


def classify(chunk: Chunk) -> Json:
    message_stop = is_message_stop(chunk)
    provider_error = is_provider_error(chunk)
    return {
        "message_stop": message_stop,
        "ping": is_anthropic_ping_chunk(chunk),
        "content_delta": is_anthropic_content_delta_chunk(chunk),
        "provider_error": provider_error,
        "terminal": message_stop or provider_error,
    }


def error_projection(chunk: Chunk) -> Json:
    parsed = parse_anthropic_error_event(chunk)
    error: Json = (
        None if parsed is None else {"type": parsed[0], "message": parsed[1], "status": parsed[2]}
    )
    details = JSON.validate_python(parse_anthropic_refusal_stop_details(chunk))
    return {
        "terminal": is_message_stop(chunk) or is_provider_error(chunk),
        "error": error,
        "refusal_stop_details": details,
    }


def run(input_value: Input) -> Json:
    action = input_value["action"]
    if action == "incomplete_error_literal":
        return incomplete_literal()
    if action == "response_as_sse_events":
        response = input_value.get("response")
        if response is None:
            raise ValueError("the response field is required")
        return render_response(response)
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
