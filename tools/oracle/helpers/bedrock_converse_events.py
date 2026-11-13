from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast
from uuid import UUID

import litellm
from litellm.llms.bedrock.chat import invoke_handler
from litellm.llms.bedrock.chat.invoke_handler import AWSEventStreamDecoder
from litellm.types.utils import ModelResponseStream

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class ConverseStep(TypedDict):
    function: Literal["converse"]
    chunk: dict[str, Json]


class ChunkStep(TypedDict):
    function: Literal["chunk"]
    chunk: dict[str, Json]


class CheckEmptyStep(TypedDict):
    function: Literal["check_empty"]
    content_blocks: list[Json]


class StateStep(TypedDict):
    function: Literal["state"]


Step: TypeAlias = ConverseStep | ChunkStep | CheckEmptyStep | StateStep


class Scenario(TypedDict):
    model: str
    json_mode: bool
    mappings: NotRequired[dict[str, str]]
    ids: list[str]
    steps: list[Step]


class Input(TypedDict):
    scenarios: list[Scenario]


class ChunkParser(Protocol):
    content_blocks: list[Json]
    tool_calls_index: int | None
    response_id: str | None
    json_mode: bool | None
    _current_tool_name: str | None
    _thinking_ran: bool
    _provider_reasoning_tokens: int | None

    def converse_chunk_parser(self, chunk_data: dict[str, Json]) -> ModelResponseStream: ...

    def _chunk_parser(
        self, chunk_data: dict[str, Json]
    ) -> ModelResponseStream | dict[str, Json]: ...

    def check_empty_tool_call_args(self) -> bool: ...


class FixedUuid:
    def __init__(self, values: list[UUID]) -> None:
        self.values = values

    def __call__(self) -> UUID:
        return self.values.pop(0)


def _converse_view(chunk: ModelResponseStream) -> Json:
    dumped = cast(dict[str, Json], chunk.model_dump())  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2096
    choices = cast(list[Json], dumped["choices"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103
    choice = cast(dict[str, Json], choices[0])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103
    delta = cast(dict[str, Json], choice["delta"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103
    return {
        "id": dumped["id"],
        "model": dumped["model"],
        "finish_reason": choice.get("finish_reason"),
        "index": choice["index"],
        "delta": {
            "content": delta.get("content"),
            "role": delta.get("role"),
            "tool_calls": delta.get("tool_calls"),
            "provider_specific_fields": delta.get("provider_specific_fields"),
            "thinking_blocks": delta.get("thinking_blocks"),
            "reasoning_content": delta.get("reasoning_content"),
        },
        "usage": dumped.get("usage"),
        "provider_specific_fields": dumped.get("provider_specific_fields"),
    }


def _generic_view(chunk: dict[str, Json]) -> Json:
    return {
        "text": chunk["text"],
        "tool_use": chunk.get("tool_use"),
        "is_finished": chunk["is_finished"],
        "finish_reason": chunk["finish_reason"],
        "usage": chunk.get("usage"),
        "index": chunk.get("index"),
        "provider_specific_fields": chunk.get("provider_specific_fields"),
    }


def _state_view(decoder: ChunkParser) -> Json:
    return {
        "content_blocks": cast(Json, list(decoder.content_blocks)),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_handler.py#L409
        "tool_calls_index": decoder.tool_calls_index,
        "response_id": decoder.response_id,
        "current_tool_name": decoder._current_tool_name,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_handler.py#L412
        "thinking_ran": decoder._thinking_ran,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_handler.py#L413
        "provider_reasoning_tokens": decoder._provider_reasoning_tokens,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_handler.py#L414
        "json_mode": decoder.json_mode,
    }


def _converse_step(decoder: ChunkParser, chunk: dict[str, Json]) -> Json:
    try:
        projected = decoder.converse_chunk_parser(chunk)
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_handler.py#L688
        return {"failed": True}
    return {"ok": {"converse": _converse_view(projected)}}


def _chunk_step(decoder: ChunkParser, chunk: dict[str, Json]) -> Json:
    try:
        projected = decoder._chunk_parser(chunk)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_handler.py#L690
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_handler.py#L741
        return {"failed": True}
    match projected:
        case ModelResponseStream():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103
            return {"ok": {"converse": _converse_view(projected)}}
        case dict():
            return {"ok": {"generic": _generic_view(projected)}}
    raise ValueError("the Bedrock chunk projection is unknown")


def _check_empty_step(decoder: ChunkParser, blocks: list[Json]) -> Json:
    decoder.content_blocks = blocks
    try:
        empty = decoder.check_empty_tool_call_args()
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_handler.py#L420
        return {"failed": True}
    return {"ok": {"boolean": empty}}


def _step_view(decoder: ChunkParser, step: Step) -> Json:
    match step["function"]:
        case "converse":
            return _converse_step(decoder, step["chunk"])
        case "chunk":
            return _chunk_step(decoder, step["chunk"])
        case "check_empty":
            return _check_empty_step(decoder, step["content_blocks"])
        case "state":
            return {"ok": {"state": _state_view(decoder)}}


def _scenario_view(scenario: Scenario) -> Json:
    invoke_handler.uuid.uuid4 = FixedUuid([UUID(value) for value in scenario["ids"]])  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_handler.py#L11
    decoder = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_handler.py#L407
        ChunkParser,
        AWSEventStreamDecoder(model=scenario["model"], json_mode=scenario["json_mode"]),
    )
    mappings = scenario.get("mappings")
    litellm.bedrock_tool_name_mappings.cache_dict.clear()  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L2383
    if mappings is not None:
        litellm.bedrock_tool_name_mappings.cache_dict.update(mappings)  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L2383
    return [_step_view(decoder, step) for step in scenario["steps"]]


def run(input_value: Input) -> Json:
    rendered = [_scenario_view(scenario) for scenario in input_value["scenarios"]]
    return {"scenarios": rendered}
