import json
import logging
import uuid
from typing import TYPE_CHECKING, Protocol, TypeAlias, cast
from unittest import mock

import litellm.types.utils as types_utils
from litellm._logging import verbose_logger
from litellm.llms.base_llm.base_model_iterator import BaseModelResponseIterator
from litellm.llms.base_llm.base_utils import map_developer_role_to_system_role
from litellm.llms.mistral.batches.transformation import MistralBatchesConfig
from litellm.llms.openai.chat.gpt_transformation import OpenAIGPTConfig

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.DEBUG)
        self.lines: list[Json] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.lines.append({"level": record.levelname, "message": record.getMessage()})


class FixtureStream:
    def __init__(self) -> None:
        self.reads = 0

    def __iter__(self) -> "FixtureStream":
        return self

    def __next__(self) -> bytes:
        self.reads += 1
        raise StopIteration


class AddsResponseFormat(Protocol):
    def _add_response_format_to_tools(
        self,
        optional_params: dict[str, Json],
        value: dict[str, Json],
        *,
        is_response_format_supported: bool,
        enforce_tool_choice: bool = True,
    ) -> dict[str, Json]: ...


class Recorder:
    def __init__(self, kind: str, calls: list[Json]) -> None:
        self.kind = kind
        self.calls = calls

    def __call__(self, **fields: Json) -> None:
        self.calls.append({"kind": self.kind, "fields": fields})


def object_of(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def as_json(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(value)))


def response_format(input_value: dict[str, Json]) -> Json:
    params = object_of(input_value["optional_params"])
    value = object_of(input_value["value"])
    supported = input_value["supported"] is True
    enforce = input_value["enforce_tool_choice"]
    config = cast(AddsResponseFormat, OpenAIGPTConfig())
    # the pinned protected method is the behavior under test
    add = config._add_response_format_to_tools  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    if enforce is None:
        return add(params, value, is_response_format_supported=supported)
    enforced = enforce is True
    return add(params, value, is_response_format_supported=supported, enforce_tool_choice=enforced)


def iterator_state(mode: Json) -> Json:
    stream = FixtureStream()
    if mode == "default":
        iterator = BaseModelResponseIterator(stream, sync_stream=True)
    else:
        explicit = object_of(mode)["explicit"]
        json_mode = explicit if isinstance(explicit, bool) else None
        iterator = BaseModelResponseIterator(stream, sync_stream=True, json_mode=json_mode)
    attributes = cast(dict[str, object], vars(iterator))
    handles = {id(attributes["streaming_response"]), id(attributes["response_iterator"])}
    held = attributes["streaming_response"] is stream
    return {
        "handles": len(handles) if held else -1,
        "reads": stream.reads,
        "json_mode": iterator.json_mode,
        "http_response": None if iterator.http_response is None else "set",
    }


def tool_call(record: dict[str, Json]) -> Json:
    calls: list[Json] = []
    custom = Recorder("custom", calls)
    function = Recorder("function", calls)
    with (
        mock.patch.object(types_utils, "ChatCompletionMessageCustomToolCall", custom),
        mock.patch.object(types_utils, "ChatCompletionMessageToolCall", function),
    ):
        _ = types_utils.chat_completion_tool_call_from_dict(record)
    return calls[0]


def outcome(input_value: dict[str, Json]) -> Json:
    function = input_value["function"]
    if function == "response_format":
        return response_format(input_value)
    if function == "developer_role":
        messages = cast("list[AllMessageValues]", input_value["messages"])
        return as_json(map_developer_role_to_system_role(messages))
    if function == "iterator_state":
        return iterator_state(input_value["json_mode"])
    if function == "batch_initializer":
        return {"attributes": len(vars(MistralBatchesConfig()))}
    if function == "chat_id":
        fixed = uuid.UUID(str(input_value["uuid"]))
        with mock.patch.object(types_utils.uuid, "uuid4", return_value=fixed):
            return types_utils._generate_id()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    if function == "tool_call":
        return tool_call(object_of(input_value["record"]))
    raise ValueError(f"unknown provider_message_primitives function: {function}")


def run(input_value: dict[str, Json]) -> Json:
    capture = Capture()
    level = verbose_logger.level
    # LiteLLM's own handler writes DEBUG records to stdout, which carries the helper JSON
    handlers = verbose_logger.handlers
    verbose_logger.handlers = [capture]
    verbose_logger.setLevel(logging.DEBUG)
    try:
        result = outcome(input_value)
    finally:
        verbose_logger.handlers = handlers
        verbose_logger.setLevel(level)
    return {"result": result, "logs": capture.lines}
