import asyncio
import json
from copy import deepcopy
from typing import Literal, TypedDict, cast

from litellm.llms.databricks.streaming_utils import ModelResponseIterator
from litellm.types.utils import GenericStreamingChunk
from pydantic import JsonValue
from pydantic_core import to_jsonable_python


class LineEvent(TypedDict):
    kind: Literal["line"]
    line: str


class ErrorEvent(TypedDict):
    kind: Literal["value_error", "receive_error"]
    message: str


Event = LineEvent | ErrorEvent


class Input(TypedDict):
    mode: str
    events: list[Event]


class Stream:
    def __init__(self, events: list[Event]) -> None:
        self.events = events

    def __iter__(self) -> "Stream":
        return self

    def __aiter__(self) -> "Stream":
        return self

    def __next__(self) -> str:
        if not self.events:
            raise StopIteration
        return receive(self.events.pop(0))

    async def __anext__(self) -> str:
        if not self.events:
            raise StopAsyncIteration
        return receive(self.events.pop(0))


def receive(event: Event) -> str:
    if event["kind"] == "line":
        return event["line"]
    if event["kind"] == "value_error":
        raise ValueError(event["message"])
    raise OSError(event["message"])


def compact_arguments(arguments: str) -> str:
    try:
        parsed = cast(JsonValue, json.loads(arguments))
    except ValueError:
        return arguments
    return json.dumps(parsed, separators=(",", ":"), sort_keys=True, ensure_ascii=False)


def chunk_value(chunk: GenericStreamingChunk) -> dict[str, object]:
    tool_use = cast(JsonValue, to_jsonable_python(chunk.get("tool_use")))
    function = tool_use.get("function") if isinstance(tool_use, dict) else None
    if isinstance(function, dict) and isinstance(arguments := function.get("arguments"), str):
        function["arguments"] = compact_arguments(arguments)
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
    iterator = ModelResponseIterator(Stream(events), sync_stream=False)
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
    iterator = ModelResponseIterator(Stream(events), sync_stream=True)
    output: list[dict[str, object]] = []
    while True:
        try:
            chunk = next(iterator)
        except StopIteration:
            output.append({"kind": "end"})
            break
        except (RuntimeError, OSError) as error:
            output.append({"kind": "error", "value": str(error)})
            break
        output.append({"kind": "chunk", "value": chunk_value(chunk)})
    return output


def run(input_value: Input) -> list[dict[str, object]]:
    events = deepcopy(input_value["events"])
    if input_value["mode"] == "async":
        return asyncio.run(collect_async(events))
    return collect_sync(events)
