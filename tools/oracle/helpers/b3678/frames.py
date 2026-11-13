import json
from binascii import crc32
from typing import Final, TypedDict

from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

PRELUDE: Final = 12
MESSAGE_CRC: Final = 4
MIN_FRAME: Final = 16
STRING_HEADER: Final = 7


class Frame(TypedDict):
    event_type: str
    payload: JsonValue


class Decoded(TypedDict):
    headers: dict[str, str]
    payload: JsonValue


class DecodedStream(TypedDict):
    frames: list[Decoded]
    trailing: str


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
