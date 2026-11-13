import asyncio
from collections.abc import AsyncIterator, Callable, Iterator
from typing import Literal, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.llms.sap.chat import handler
from litellm.llms.sap.chat.handler import (
    AsyncSAPStreamIterator,
    GenAIHubOrchestration,
    GenAIHubOrchestrationError,
    SAPStreamIterator,
    _StreamParser,  # pyright: ignore[reportPrivateUsage]
)
from litellm.types.llms.openai import OpenAIChatCompletionChunk
from pydantic import BaseModel, ConfigDict, JsonValue

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Failure: TypeAlias = Literal["read", "os"]


class Request(BaseModel):
    model_config = ConfigDict(extra="forbid")
    operation: Literal["event", "lines", "flag"]
    params: dict[str, JsonValue]


class EventParams(BaseModel):
    model_config = ConfigDict(extra="forbid")
    event: JsonValue
    now: int


class LinesParams(BaseModel):
    model_config = ConfigDict(extra="forbid")
    lines: list[str]
    modes: list[Literal["sync", "async"]]
    now: int
    failure: Failure | None = None


class FlagParams(BaseModel):
    model_config = ConfigDict(extra="forbid")
    body: JsonValue


ADD_STREAM = cast(
    "Callable[[object, Object, object, bool], Object]",
    GenAIHubOrchestration._add_stream_param_to_request_body,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
)


def chunk_json(chunk: OpenAIChatCompletionChunk) -> Json:
    return cast(Json, chunk.model_dump(mode="json", exclude_none=True))


def source(lines: list[str], failure: Failure | None) -> Iterator[str]:
    yield from lines
    if failure == "read":
        raise httpx.ReadError("read failed")
    if failure == "os":
        raise OSError("io failed")


async def source_async(lines: list[str], failure: Failure | None) -> AsyncIterator[str]:
    for line in lines:
        yield line
    if failure == "read":
        raise httpx.ReadError("read failed")
    if failure == "os":
        raise OSError("io failed")


def event_output(params: EventParams) -> Json:
    event = cast(Object, params.event)
    with patch.object(handler, "_now_ts", return_value=params.now):
        chunk = _StreamParser.to_openai_chunk(event)  # pyright: ignore[reportUnknownMemberType]
    return {"chunk": None if chunk is None else chunk_json(chunk)}


def sync_lines(params: LinesParams) -> Json:
    iterator = SAPStreamIterator(source(params.lines, params.failure))
    chunks: list[Json] = []
    failure: Json = None
    with patch.object(handler, "_now_ts", return_value=params.now):
        try:
            for chunk in iterator:
                chunks.append(chunk_json(chunk))  # noqa: PERF401 - a failure keeps the chunks already read
        except GenAIHubOrchestrationError as error:
            failure = {"status": error.status_code}
        except Exception:  # noqa: BLE001 - the helper reports whichever error the iterator raised
            failure = {"status": None}
    return {"chunks": chunks, "failure": failure, "done": iterator._done}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


async def run_async(params: LinesParams) -> Json:
    iterator = AsyncSAPStreamIterator(source_async(params.lines, params.failure))
    chunks: list[Json] = []
    failure: Json = None
    with patch.object(handler, "_now_ts", return_value=params.now):
        try:
            async for chunk in iterator:
                chunks.append(chunk_json(chunk))  # noqa: PERF401 - a failure keeps the chunks already read
        except GenAIHubOrchestrationError as error:
            failure = {"status": error.status_code}
        except Exception:  # noqa: BLE001 - the helper reports whichever error the iterator raised
            failure = {"status": None}
    return {"chunks": chunks, "failure": failure, "done": iterator._done}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def async_lines(params: LinesParams) -> Json:
    return asyncio.run(run_async(params))


def trace(params: LinesParams, mode: Literal["sync", "async"]) -> Json:
    return sync_lines(params) if mode == "sync" else async_lines(params)


def lines_output(params: LinesParams) -> Json:
    return {"traces": [trace(params, mode) for mode in params.modes]}


def flag_output(params: FlagParams) -> Json:
    instance = object.__new__(GenAIHubOrchestration)
    body = cast(Object, params.body)
    result = ADD_STREAM(instance, body, None, False)  # noqa: FBT003 - the fake_stream flag is positional
    return {"body": cast(Json, result)}


def run(input_value: dict[str, Json]) -> Json:
    request = Request.model_validate(input_value)
    if request.operation == "event":
        return event_output(EventParams.model_validate(request.params))
    if request.operation == "lines":
        return lines_output(LinesParams.model_validate(request.params))
    return flag_output(FlagParams.model_validate(request.params))
