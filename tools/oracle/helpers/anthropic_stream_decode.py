import asyncio
from collections.abc import AsyncIterator
from typing import Literal, NotRequired, Protocol, TypedDict, runtime_checkable

from litellm.llms.anthropic.chat.handler import ModelResponseIterator
from pydantic import JsonValue, TypeAdapter


class TextStep(TypedDict):
    function: Literal["text"]
    value: str


class BytesStep(TypedDict):
    function: Literal["bytes"]
    value: list[int]


class ValueErrorStep(TypedDict):
    function: Literal["value_error"]
    message: str


class OtherErrorStep(TypedDict):
    function: Literal["other_error"]
    message: str


Step = TextStep | BytesStep | ValueErrorStep | OtherErrorStep
PassthroughStep = TextStep | BytesStep


class Input(TypedDict):
    ids: list[str]
    steps: list[Step]
    passthrough: list[PassthroughStep]


@runtime_checkable
class HasDump(Protocol):
    def model_dump_json(self) -> str: ...


@runtime_checkable
class HasLength(Protocol):
    def __len__(self) -> int: ...


class SyncSource(Protocol):
    def __iter__(self) -> "SyncSource": ...

    def __next__(self) -> HasDump | HasLength: ...


class AsyncSource(Protocol):
    def __aiter__(self) -> "AsyncSource": ...

    async def __anext__(self) -> HasDump | HasLength: ...


class DeltaView(TypedDict):
    content: JsonValue
    tool_calls: JsonValue | None
    provider_specific_fields: JsonValue | None


class ChoiceView(TypedDict):
    finish_reason: str | None
    delta: DeltaView


class ChunkView(TypedDict):
    id: str
    model: str | None
    usage: NotRequired[JsonValue | None]
    choices: list[ChoiceView]


class FakeSyncSource:
    def __init__(self, steps: list[Step]) -> None:
        self.steps = list(steps)

    def __iter__(self) -> "FakeSyncSource":
        return self

    def __next__(self) -> str | bytes:
        if not self.steps:
            raise StopIteration
        return _step_value(self.steps.pop(0))


class FakeAsyncSource:
    def __init__(self, steps: list[Step]) -> None:
        self.sync_source = FakeSyncSource(steps)

    def __aiter__(self) -> "FakeAsyncSource":
        return self

    async def __anext__(self) -> str | bytes:
        if not self.sync_source.steps:
            raise StopAsyncIteration
        return next(self.sync_source)


def _step_value(step: Step) -> str | bytes:
    match step["function"]:
        case "text":
            return step["value"]
        case "bytes":
            return bytes(step["value"])
        case "value_error":
            raise ValueError(step["message"])
        case "other_error":
            raise RuntimeError(step["message"])


def _iterator(
    input_value: Input,
    source: FakeSyncSource | FakeAsyncSource,
) -> ModelResponseIterator:
    iterator = ModelResponseIterator(
        source,
        sync_stream=False,
        json_mode=None,
        speed=None,
        tool_name_reverse_map={},
    )
    iterator.response_id = "chatcmpl-" + input_value["ids"][0]
    return iterator


def _event_view(item: HasDump) -> dict[str, JsonValue]:
    view = TypeAdapter(ChunkView).validate_json(item.model_dump_json())
    delta = view["choices"][0]["delta"]
    return {
        "id": view["id"],
        "model": view["model"],
        "usage": view.get("usage"),
        "content": delta["content"],
        "finish_reason": view["choices"][0]["finish_reason"],
        "tool_calls": delta["tool_calls"],
        "provider_specific_fields": delta["provider_specific_fields"],
    }


def _item_view(item: HasDump | HasLength) -> dict[str, JsonValue]:
    match item:
        case HasDump():
            return {"event": _event_view(item)}
        case HasLength():
            return {"generic": TypeAdapter(dict[str, JsonValue]).validate_python(item)}


def _passthrough_item(item: HasDump) -> dict[str, JsonValue]:
    dumped = TypeAdapter(dict[str, JsonValue]).validate_json(item.model_dump_json())
    if "usage" not in dumped:
        return {"default": dumped["id"]}
    return {"event": _event_view(item)}


def _passthrough_chunk(step: PassthroughStep) -> str:
    match step["function"]:
        case "text":
            return step["value"]
        case "bytes":
            return bytes(step["value"]).decode("utf-8")


def _run_sync(input_value: Input) -> JsonValue:
    source: SyncSource = _iterator(input_value, FakeSyncSource(input_value["steps"]))
    items: list[JsonValue] = []
    errors = 0
    while True:
        try:
            item = next(source)
        except StopIteration:
            return {"items": items, "errors": errors}
        except (AttributeError, KeyError, RuntimeError, TypeError, ValueError):
            errors += 1
        else:
            items.append(_item_view(item))


async def _pull(source: AsyncSource) -> AsyncIterator[HasDump | HasLength | None]:
    iterator = source.__aiter__()
    while True:
        try:
            yield await iterator.__anext__()
        except StopAsyncIteration:
            return
        except (AttributeError, KeyError, RuntimeError, TypeError, ValueError):
            yield None


async def _drain_async(source: AsyncSource) -> JsonValue:
    items: list[JsonValue] = []
    errors = 0
    async for item in _pull(source):
        if item is None:
            errors += 1
        else:
            items.append(_item_view(item))
    return {"items": items, "errors": errors}


def _run_async(input_value: Input) -> JsonValue:
    source: AsyncSource = _iterator(input_value, FakeAsyncSource(input_value["steps"]))
    return asyncio.run(_drain_async(source))


def _run_passthrough(input_value: Input) -> JsonValue:
    iterator = _iterator(input_value, FakeSyncSource([]))
    items: list[JsonValue] = []
    errors = 0
    for index in range(len(input_value["passthrough"])):
        try:
            chunk = iterator.convert_str_chunk_to_generic_chunk(
                _passthrough_chunk(input_value["passthrough"][index])
            )
        except (AttributeError, KeyError, RuntimeError, TypeError, ValueError):
            errors += 1
        else:
            items.append(_passthrough_item(chunk))
    return {"items": items, "errors": errors}


def run(input_value: Input) -> JsonValue:
    return {
        "sync": _run_sync(input_value),
        "async": _run_async(input_value),
        "passthrough": _run_passthrough(input_value),
    }
