from copy import deepcopy
from typing import Literal, TypeAlias, TypedDict
from uuid import UUID

import litellm.types.utils
from litellm.llms.anthropic.chat.handler import ModelResponseIterator
from litellm.llms.anthropic.common_utils import AnthropicError
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue, TypeAdapter


class DeltaView(TypedDict):
    content: str
    provider_specific_fields: JsonValue | None
    tool_calls: list[JsonValue] | None
    thinking_blocks: list[JsonValue] | None
    reasoning_content: str | None


class ChoiceView(TypedDict):
    index: int
    finish_reason: str | None
    delta: DeltaView


class ChunkView(TypedDict):
    id: str
    model: str | None
    choices: list[ChoiceView]
    usage: JsonValue | None


class ChunkStep(TypedDict):
    function: Literal["chunk"]
    chunk: dict[str, JsonValue]


class AccumulatedStep(TypedDict):
    function: Literal["accumulated"]
    fragment: str
    is_final: bool


class StateStep(TypedDict):
    function: Literal["state"]


Step: TypeAlias = ChunkStep | AccumulatedStep | StateStep


class Input(TypedDict):
    json_mode: bool | None
    tool_name_reverse_map: dict[str, str]
    ids: list[str]
    steps: list[Step]


class FixedUuid:
    def __init__(self, values: list[UUID]) -> None:
        self.values = values

    def __call__(self) -> UUID:
        return self.values.pop(0)


def _iterator(input_value: Input) -> ModelResponseIterator:
    litellm.types.utils.uuid.uuid4 = FixedUuid(  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L122
        [UUID(value) for value in input_value["ids"]]
    )
    return ModelResponseIterator(
        None,
        sync_stream=False,
        json_mode=input_value["json_mode"],
        speed=None,
        tool_name_reverse_map=input_value["tool_name_reverse_map"],
    )


def _chunk_view(chunk: ModelResponseStream) -> JsonValue:
    view = TypeAdapter(ChunkView).validate_json(chunk.model_dump_json())
    delta = view["choices"][0]["delta"]
    delta_view: dict[str, JsonValue] = {
        "content": delta["content"],
        "provider_specific_fields": delta["provider_specific_fields"],
    }
    if delta["tool_calls"] is not None:
        delta_view["tool_calls"] = delta["tool_calls"]
    if delta["thinking_blocks"] is not None:
        delta_view["thinking_blocks"] = delta["thinking_blocks"]
    if delta["reasoning_content"] is not None:
        delta_view["reasoning_content"] = delta["reasoning_content"]
    return {
        "id": view["id"],
        "model": view["model"],
        "choices": [
            {
                "index": view["choices"][0]["index"],
                "finish_reason": view["choices"][0]["finish_reason"],
                "delta": delta_view,
            }
        ],
        "usage": view["usage"],
    }


def _state_view(iterator: ModelResponseIterator) -> JsonValue:
    server_inputs = iterator._server_tool_inputs  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L599
    view: dict[str, JsonValue] = {
        "current_content_block_type": iterator.current_content_block_type,
        "current_server_tool_id": iterator._current_server_tool_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L601
        "is_response_format_tool": iterator.is_response_format_tool,
        "converted_response_format_tool": iterator.converted_response_format_tool,
        "tool_index": iterator.tool_index,
        "served_model": iterator.served_model,
        "container_id": iterator._container_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L602
        "server_tool_inputs": TypeAdapter(JsonValue).validate_python(server_inputs),
    }
    return deepcopy(view)


def _run_step(iterator: ModelResponseIterator, step: Step) -> JsonValue:
    try:
        match step["function"]:
            case "chunk":
                parsed = iterator.chunk_parser(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L811
                    chunk=step["chunk"]
                )
                return {"chunk": _chunk_view(parsed)}
            case "accumulated":
                accumulated = iterator._handle_accumulated_json_chunk(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L1118
                    data_str=step["fragment"], is_final=step["is_final"]
                )
                return {"accumulated": None if accumulated is None else _chunk_view(accumulated)}
            case "state":
                return {"state": _state_view(iterator)}
    except AnthropicError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L309
        return {"error": {"status": error.status_code}}
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L811
        return {"failed": True}


def run(input_value: Input) -> JsonValue:
    iterator = _iterator(input_value)
    return {"steps": [_run_step(iterator, step) for step in input_value["steps"]]}
