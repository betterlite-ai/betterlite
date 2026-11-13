import json
from collections.abc import Generator, Iterator
from contextlib import contextmanager, suppress
from typing import TypeAlias, cast
from unittest.mock import patch

from botocore.eventstream import EventStreamBuffer, EventStreamMessage
from botocore.loaders import Loader
from litellm.llms.bedrock.chat.invoke_handler import _EventStreamTally  # pyright: ignore[reportPrivateUsage]
from litellm.llms.bedrock.common_utils import (
    BedrockError,
    BedrockEventStreamDecoderBase,
    get_anthropic_beta_from_headers,  # pyright: ignore[reportUnknownVariableType]
    get_bedrock_response_stream_shape,
)
from litellm.types.llms.bedrock import ServerSentEvent
from pydantic import BaseModel, ConfigDict, JsonValue

Json: TypeAlias = JsonValue


class Request(BaseModel):
    model_config = ConfigDict(extra="forbid")
    operation: str
    params: dict[str, Json]


class FramesCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    chunks: list[str]


class StreamCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    chunks: list[str]
    response_headers: dict[str, str] | None
    shape_loaded: bool


class BetaCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    headers: dict[str, Json]


class SseCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    event: str | None
    data: str | None
    id: str | None
    retry: int | None


def events(chunks: list[str], tally: _EventStreamTally) -> Iterator[EventStreamMessage]:
    # The loop of AWSEventStreamDecoder.iter_bytes up to the point where it parses a message.
    buffer = EventStreamBuffer()
    for chunk in chunks:
        data = bytes.fromhex(chunk)
        buffer.add_data(data)  # pyright: ignore[reportUnknownMemberType]
        tally.add_chunk(data)
        yield from buffer


def describe(error: Exception) -> Json:
    if isinstance(error, BedrockError):
        return {"kind": "bedrock", "status": error.status_code, "message": error.message}
    return {"kind": "invalid", "status": None, "message": None}


def header_json(value: object) -> Json:
    if isinstance(value, bool):
        return {"type": "bool", "value": value}
    if isinstance(value, int):
        return {"type": "int", "value": value}
    if isinstance(value, bytes):
        return {"type": "bytes", "value": value.hex()}
    if isinstance(value, str):
        return {"type": "string", "value": value}
    raise TypeError(f"unexpected header value {value!r}")


def frames(call: FramesCall) -> Json:
    decoded: list[Json] = []
    failure: Json = None
    try:
        for event in events(call.chunks, _EventStreamTally()):
            headers = cast(dict[str, object], event.headers)  # pyright: ignore[reportUnknownMemberType]
            decoded.append(
                {
                    "headers": {name: header_json(value) for name, value in headers.items()},
                    "payload": event.payload.hex(),  # pyright: ignore[reportUnknownMemberType]
                    "total_length": event.prelude.total_length,  # pyright: ignore[reportUnknownMemberType]
                }
            )
    except Exception as error:  # noqa: BLE001 - the helper reports whichever error botocore raised
        failure = describe(error)
    return {"events": decoded, "failure": failure}


@contextmanager
def response_stream_shape(*, loaded: bool) -> Generator[None]:
    get_bedrock_response_stream_shape.cache_clear()
    try:
        if loaded:
            yield
        else:
            with patch.object(
                Loader, "load_service_model", side_effect=OSError("no bedrock-runtime model")
            ):
                yield
    finally:
        get_bedrock_response_stream_shape.cache_clear()


def stream(call: StreamCall) -> Json:
    decoder = BedrockEventStreamDecoderBase()
    tally = _EventStreamTally()
    texts: list[Json] = []
    failure: Json = None
    with response_stream_shape(loaded=call.shape_loaded):
        try:
            for event in events(call.chunks, tally):
                tally.add_event(event)
                texts.append(decoder._parse_message_from_event(event))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            undecoded = tally.undecoded_stream_error(call.response_headers)
            if undecoded is not None:
                raise undecoded
        except Exception as error:  # noqa: BLE001 - the helper reports whichever error LiteLLM raised
            failure = describe(error)
    return {"texts": texts, "failure": failure}


def beta(call: BetaCall) -> Json:
    return [name for name in get_anthropic_beta_from_headers(call.headers)]  # noqa: C416


def sse(call: SseCall) -> Json:
    record = ServerSentEvent(event=call.event, data=call.data, id=call.id, retry=call.retry)
    parsed: Json = {"ok": False, "value": None}
    with suppress(json.JSONDecodeError):
        parsed = {"ok": True, "value": cast(Json, record.json())}
    return {
        "event": record.event,
        "data": record.data,
        "id": record.id,
        "retry": record.retry,
        "repr": repr(record),
        "json": parsed,
    }


def run(input_value: dict[str, Json]) -> Json:
    request = Request.model_validate(input_value)
    match request.operation:
        case "frames":
            return frames(FramesCall.model_validate(request.params))
        case "stream":
            return stream(StreamCall.model_validate(request.params))
        case "beta":
            return beta(BetaCall.model_validate(request.params))
        case "sse":
            return sse(SseCall.model_validate(request.params))
        case other:
            raise ValueError(f"unknown operation {other}")
