from typing import Final, Literal, TypedDict

import litellm
from litellm.llms.anthropic.chat.guardrail_translation.handler import (
    AnthropicMessagesHandler,
    _tool_call_shapes,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L269-L279
)
from litellm.proxy.policy_engine.pipeline_executor import UndeliverableStreamRewrite
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)


class BytesChunk(TypedDict):
    bytes: str


class EventChunk(TypedDict):
    event: dict[str, JsonValue]


ChunkInput = BytesChunk | EventChunk

BYTES_CHUNK: Final[TypeAdapter[BytesChunk]] = TypeAdapter(BytesChunk)
EVENT_CHUNK: Final[TypeAdapter[EventChunk]] = TypeAdapter(EventChunk)


class ShapeInput(TypedDict):
    name: str | None
    arguments: str


class TextScenario(TypedDict):
    responses_so_far: list[ChunkInput]
    rewritten_text: str
    guardrail_name: str


class ToolScenario(TypedDict):
    responses_so_far: list[ChunkInput]
    pre_guardrail_tool_calls: list[ShapeInput]
    post_guardrail_tool_calls: list[ShapeInput]
    guardrail_name: str


class UsageScenario(TypedDict):
    responses_so_far: list[ChunkInput]
    request_data: dict[str, JsonValue] | None


class MessageProjection(TypedDict):
    role: str
    content: str | None


class ChoiceProjection(TypedDict):
    finish_reason: str | None
    message: MessageProjection


class UsageProjection(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ResponseProjection(TypedDict):
    object: str
    model: str
    choices: list[ChoiceProjection]
    usage: UsageProjection


class TextCall(TypedDict):
    function: Literal["text_rewrite"]
    scenarios: list[TextScenario]


class ToolCall(TypedDict):
    function: Literal["tool_rewrite"]
    scenarios: list[ToolScenario]


class UsageCall(TypedDict):
    function: Literal["usage_response"]
    scenarios: list[UsageScenario]


Input = TextCall | ToolCall | UsageCall
Item = bytes | dict[str, JsonValue]

RESPONSE: Final[TypeAdapter[ResponseProjection]] = TypeAdapter(ResponseProjection)


def chunk_items(chunks: list[ChunkInput]) -> list[Item]:
    return [
        BYTES_CHUNK.validate_python(chunk)["bytes"].encode("utf-8")
        if "bytes" in chunk
        else EVENT_CHUNK.validate_python(chunk)["event"]
        for chunk in chunks
    ]


def encoded_item(item: Item) -> JsonValue:
    match item:
        case bytes():
            return {"bytes": item.decode("utf-8")}
        case dict():
            return {"event": item}


def text_error(items: list[Item], scenario: TextScenario) -> str | None:
    write_text = AnthropicMessagesHandler._write_ended_stream_text_rewrite  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L1413-L1448
    try:
        write_text(
            responses_so_far=items,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L1414-L1419
            rewritten_text=scenario["rewritten_text"],
            guardrail_name=scenario["guardrail_name"],
        )
    except UndeliverableStreamRewrite as failure:
        return str(failure)
    return None


def tool_error(items: list[Item], scenario: ToolScenario) -> str | None:
    pre = _tool_call_shapes(
        [
            {"function": {"name": shape["name"], "arguments": shape["arguments"]}}
            for shape in scenario["pre_guardrail_tool_calls"]
        ]
    )
    post = _tool_call_shapes(
        [
            {"function": {"name": shape["name"], "arguments": shape["arguments"]}}
            for shape in scenario["post_guardrail_tool_calls"]
        ]
    )
    write_tools = AnthropicMessagesHandler._write_ended_stream_tool_call_rewrites  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L1450-L1515
    try:
        write_tools(
            responses_so_far=items,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L1452-L1458
            pre_guardrail_tool_calls=pre,
            post_guardrail_tool_calls=post,
            guardrail_name=scenario["guardrail_name"],
        )
    except UndeliverableStreamRewrite as failure:
        return str(failure)
    return None


def chunks_answer(items: list[Item], error: str | None) -> JsonValue:
    encoded = [encoded_item(item) for item in items]
    return {"chunks": encoded, "error": error}


def text_answer(scenario: TextScenario) -> JsonValue:
    items = chunk_items(scenario["responses_so_far"])
    error = text_error(items, scenario)
    return chunks_answer(items, error)


def tool_answer(scenario: ToolScenario) -> JsonValue:
    items = chunk_items(scenario["responses_so_far"])
    error = tool_error(items, scenario)
    return chunks_answer(items, error)


def usage_answer(scenario: UsageScenario) -> JsonValue:
    items = chunk_items(scenario["responses_so_far"])
    request_data = scenario["request_data"]
    response = AnthropicMessagesHandler._build_streaming_usage_response(items, request_data)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L311-L325
    if response is None:
        return {"response": None}
    projected = RESPONSE.validate_json(response.model_dump_json())
    return {"response": JSON.validate_python(projected)}


def answers(call: Input) -> list[JsonValue]:
    if call["function"] == "text_rewrite":
        return [text_answer(scenario) for scenario in call["scenarios"]]
    if call["function"] == "tool_rewrite":
        return [tool_answer(scenario) for scenario in call["scenarios"]]
    return [usage_answer(scenario) for scenario in call["scenarios"]]


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    return JSON.validate_python(answers(input_value))
