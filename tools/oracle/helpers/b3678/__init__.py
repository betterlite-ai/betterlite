import asyncio
import json
from binascii import crc32
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.llms.bedrock.passthrough.guardrail_translation.handler import (
    BedrockPassthroughGuardrailHandler,
)
from litellm.proxy._types import UserAPIKeyAuth
from pydantic import JsonValue, TypeAdapter

from b3678 import scripted

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
HANDLER: Final = BedrockPassthroughGuardrailHandler()

PRELUDE: Final = 12
MESSAGE_CRC: Final = 4
MIN_FRAME: Final = 16
STRING_HEADER: Final = 7


class RoutingInput(TypedDict):
    function: Literal["routing"]
    endpoint: str
    content_type: str


class InputTextsInput(TypedDict):
    function: Literal["input_texts"]
    endpoint: str
    data: dict[str, JsonValue]
    skip_system: bool
    skip_tool: bool
    returned_texts: NotRequired[list[str]]


class OutputTextsInput(TypedDict):
    function: Literal["output_texts"]
    endpoint: str
    response: dict[str, JsonValue]
    request_data: dict[str, JsonValue] | None
    returned_texts: NotRequired[list[str]]


class Frame(TypedDict):
    event_type: str
    payload: JsonValue


class StreamInput(TypedDict):
    function: Literal["stream"]
    frames: list[Frame]
    replace: dict[str, str]
    trailing_hex: str


class Decoded(TypedDict):
    headers: dict[str, str]
    payload: JsonValue


class DecodedStream(TypedDict):
    frames: list[Decoded]
    trailing: str


Input: TypeAlias = RoutingInput | InputTextsInput | OutputTextsInput | StreamInput


def encode_header(name: str, value: str) -> bytes:
    encoded_name = name.encode()
    encoded_value = value.encode()
    kind = bytes([STRING_HEADER])
    return (
        len(encoded_name).to_bytes(1, "big")
        + encoded_name
        + kind
        + len(encoded_value).to_bytes(2, "big")
        + encoded_value
    )


def encode_frame(frame: Frame) -> bytes:
    headers = encode_header(":event-type", frame["event_type"])
    payload = json.dumps(frame["payload"], separators=(",", ":")).encode()
    total = PRELUDE + len(headers) + len(payload) + MESSAGE_CRC
    prelude = total.to_bytes(4, "big") + len(headers).to_bytes(4, "big")
    prelude_crc = crc32(prelude).to_bytes(4, "big")
    message_crc = crc32(prelude_crc + headers + payload, crc32(prelude)).to_bytes(4, "big")
    return prelude + prelude_crc + headers + payload + message_crc


def encode_stream(frames: list[Frame], trailing_hex: str) -> bytes:
    parts = [encode_frame(frame) for frame in frames]
    parts.append(bytes.fromhex(trailing_hex))
    return b"".join(parts)


def decode_header(raw: bytes, cursor: int, end: int) -> tuple[str, str, int]:
    name_length = raw[cursor]
    cursor += 1
    name = raw[cursor : cursor + name_length].decode()
    cursor += name_length
    kind = raw[cursor]
    cursor += 1
    value_length = int.from_bytes(raw[cursor : cursor + 2], "big")
    cursor += 2
    if kind != STRING_HEADER:
        raise ValueError("only string event-stream headers are written")
    value = raw[cursor : cursor + value_length].decode()
    cursor += value_length
    if cursor > end:
        raise ValueError("an event-stream header runs past the header block")
    return name, value, cursor


def decode_frame(raw: bytes) -> Decoded:
    total = int.from_bytes(raw[0:4], "big")
    headers_length = int.from_bytes(raw[4:8], "big")
    if total != len(raw):
        raise ValueError("an event-stream frame length does not match its bytes")
    prelude_crc = int.from_bytes(raw[8:12], "big")
    if crc32(raw[0:8]) != prelude_crc:
        raise ValueError("an event-stream prelude CRC does not match")
    headers_end = PRELUDE + headers_length
    payload_end = total - MESSAGE_CRC
    message_crc = int.from_bytes(raw[payload_end:total], "big")
    if crc32(raw[8:payload_end], prelude_crc) != message_crc:
        raise ValueError("an event-stream message CRC does not match")
    headers: dict[str, str] = {}
    cursor = PRELUDE
    while cursor < headers_end:
        name, value, cursor = decode_header(raw, cursor, headers_end)
        headers[name] = value
    payload = JSON.validate_json(raw[headers_end:payload_end])
    return {"headers": headers, "payload": payload}


def decode_stream(body: bytes) -> DecodedStream:
    frames: list[Decoded] = []
    offset = 0
    while offset + MIN_FRAME <= len(body):
        total = int.from_bytes(body[offset : offset + 4], "big")
        if total < MIN_FRAME or offset + total > len(body):
            break
        frames.append(decode_frame(body[offset : offset + total]))
        offset += total
    return {"frames": frames, "trailing": body[offset:].hex()}


def routing(input_value: RoutingInput) -> JsonValue:
    kinds: dict[str, JsonValue] = {
        "is_converse": HANDLER.event_stream_endpoint_is_de_anonymizable(input_value["endpoint"]),
        "is_event_stream": HANDLER.is_event_stream_content_type(input_value["content_type"]),
        "media_type": HANDLER.event_stream_media_type(),
    }
    return JSON.validate_python(kinds)


def input_texts(input_value: InputTextsInput) -> JsonValue:
    guardrail = scripted.ScriptedGuardrail(
        input_value.get("returned_texts"),
        skip_system=input_value["skip_system"],
        skip_tool=input_value["skip_tool"],
    )
    data = dict(input_value["data"])
    data["endpoint"] = input_value["endpoint"]
    result = asyncio.run(HANDLER.process_input_messages(data=data, guardrail_to_apply=guardrail))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/passthrough/guardrail_translation/handler.py#L423-L427
    answer: dict[str, JsonValue] = {
        "texts": JSON.validate_python(guardrail.texts),
        "model": guardrail.model,
        "data": JSON.validate_python(result),
    }
    return JSON.validate_python(answer)


def output_texts(input_value: OutputTextsInput) -> JsonValue:
    guardrail = scripted.ScriptedGuardrail(
        input_value.get("returned_texts"), skip_system=False, skip_tool=False
    )
    request_data = input_value["request_data"]
    if request_data is not None:
        request_data = {**request_data, "endpoint": input_value["endpoint"]}
    result = asyncio.run(
        HANDLER.process_output_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/passthrough/guardrail_translation/handler.py#L468-L472
            response=input_value["response"],
            guardrail_to_apply=guardrail,
            request_data=request_data,
        )
    )
    answer: dict[str, JsonValue] = {
        "texts": JSON.validate_python(guardrail.texts),
        "model": guardrail.model,
        "data": JSON.validate_python(result),
    }
    return JSON.validate_python(answer)


def stream(input_value: StreamInput) -> JsonValue:
    body = encode_stream(input_value["frames"], input_value["trailing_hex"])
    hook = scripted.ScriptedHook(input_value["replace"])
    empty: dict[str, JsonValue] = {}
    rewritten = asyncio.run(
        HANDLER.de_anonymize_event_stream(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/passthrough/guardrail_translation/handler.py#L268-L273
            body_bytes=body,
            proxy_logging_obj=hook,
            user_api_key_dict=UserAPIKeyAuth(),
            data=empty,
        )
    )
    return JSON.validate_python(decode_stream(rewritten))


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "routing":
            return routing(input_value)
        case "input_texts":
            return input_texts(input_value)
        case "output_texts":
            return output_texts(input_value)
        case "stream":
            return stream(input_value)
