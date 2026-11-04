import asyncio
from copy import deepcopy
from typing import Literal, TypedDict, cast

from litellm.llms.databricks.streaming_utils import ModelResponseIterator
from litellm.types.utils import GenericStreamingChunk
from pydantic import JsonValue
from pydantic_core import to_jsonable_python


class LineEvent(TypedDict):
    kind: Literal["line"]
    line: str


class ValueErrorEvent(TypedDict):
    kind: Literal["value_error"]
    message: str


class ReceiveErrorEvent(TypedDict):
    kind: Literal["receive_error"]
    message: str


Event = LineEvent | ValueErrorEvent | ReceiveErrorEvent


class Input(TypedDict):
    mode: str
    events: list[Event]


class SyncStream:
    def __init__(self, events: list[Event]) -> None:
        self.events = events

    def __iter__(self) -> "SyncStream":
        return self

    def __next__(self) -> str:
        if not self.events:
            raise StopIteration
        event = self.events.pop(0)
        return receive(event)


class AsyncStream:
    def __init__(self, events: list[Event]) -> None:
        self.events = events

    def __aiter__(self) -> "AsyncStream":
        return self

    async def __anext__(self) -> str:
        if not self.events:
            raise StopAsyncIteration
        event = self.events.pop(0)
        return receive(event)


def receive(event: Event) -> str:
    if event["kind"] == "value_error":
        raise ValueError(event["message"])
    if event["kind"] == "receive_error":
        raise OSError(event["message"])
    return event["line"]


def chunk_value(chunk: GenericStreamingChunk) -> dict[str, object]:
    tool_use = cast(JsonValue, to_jsonable_python(chunk.get("tool_use")))
    raw_usage = cast(JsonValue, to_jsonable_python(chunk["usage"]))
    usage: JsonValue = None
    if isinstance(raw_usage, dict):
        usage = {
            key: raw_usage.get(key)
            for key in ("prompt_tokens", "completion_tokens", "total_tokens")
        }
    return {
        "text": chunk["text"],
        "tool_use": tool_use,
        "is_finished": chunk["is_finished"],
        "finish_reason": chunk["finish_reason"],
        "usage": usage,
        "index": chunk.get("index", 0),
    }


async def collect_async(events: list[Event]) -> list[dict[str, object]]:
    iterator = ModelResponseIterator(AsyncStream(events), sync_stream=False)
    iterator.__aiter__()
    output: list[dict[str, object]] = []
    while True:
        try:
            chunk = await iterator.__anext__()
        except StopAsyncIteration:
            output.append({"kind": "end"})
            break
        except RuntimeError as error:
            output.append({"kind": "error", "value": str(error)})
            break
        output.append({"kind": "chunk", "value": chunk_value(chunk)})
    return output


def collect_sync(events: list[Event]) -> list[dict[str, object]]:
    iterator = ModelResponseIterator(SyncStream(events), sync_stream=True)
    output: list[dict[str, object]] = []
    while True:
        try:
            chunk = next(iterator)
        except StopIteration:
            output.append({"kind": "end"})
            break
        except RuntimeError as error:
            output.append({"kind": "error", "value": str(error)})
            break
        except OSError as error:
            output.append({"kind": "error", "value": str(error)})
            break
        output.append({"kind": "chunk", "value": chunk_value(chunk)})
    return output


def run(input_value: Input) -> list[dict[str, object]]:
    events = deepcopy(input_value["events"])
    if input_value["mode"] == "async":
        return asyncio.run(collect_async(events))
    return collect_sync(events)
