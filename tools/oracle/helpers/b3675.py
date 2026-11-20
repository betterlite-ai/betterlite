import asyncio
from collections.abc import Callable, Coroutine, Mapping, Sequence
from typing import Final, Literal, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.anthropic.chat.guardrail_translation.handler import (
    AnthropicMessagesHandler,
    ContentBlockTextTarget,
    ExtractedInput,
    InputWriteBackTarget,
    MessageContentTarget,
    ScannedText,
    SystemBlockTextTarget,
    SystemStringTarget,
    ToolResultBlockTextTarget,
    ToolResultStringTarget,
)
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, Field, JsonValue, TypeAdapter


class Flags(BaseModel):
    skip_system_message: bool = False
    skip_tool_message: bool = False
    scan_only_tool_results: bool = False


class Scenario(BaseModel):
    message: dict[str, JsonValue] = Field(default_factory=dict)
    msg_idx: int = 0
    flags: Flags = Field(default_factory=Flags)
    block: dict[str, JsonValue] = Field(default_factory=dict)
    hoisted: dict[str, JsonValue] = Field(default_factory=dict)
    data: dict[str, JsonValue] = Field(default_factory=dict)
    content: list[JsonValue] = Field(default_factory=list)
    responses: list[str] = Field(default_factory=list)
    mappings: list[int] = Field(default_factory=list)
    response: JsonValue = None


class Input(TypedDict):
    function: Literal[
        "extract",
        "image_sources",
        "top_level_system",
        "structured_messages",
        "output_apply",
        "post_call_hook",
    ]
    scenarios: list[JsonValue]


SCENARIOS: Final[TypeAdapter[list[Scenario]]] = TypeAdapter(list[Scenario])
MODEL_RESPONSE: Final[TypeAdapter[ModelResponse]] = TypeAdapter(ModelResponse)

ExtractInput: TypeAlias = Callable[
    [Mapping[str, JsonValue], int, bool, bool, bool], ExtractedInput
]
TopLevelExtract: TypeAlias = Callable[[Mapping[str, JsonValue]], tuple[ScannedText, ...]]
ImageSources: TypeAlias = Callable[[Mapping[str, JsonValue]], tuple[str, ...]]
WriteOutputResponses: TypeAlias = Callable[
    [dict[str, JsonValue], Sequence[str], Sequence[tuple[int, int | None]]],
    Coroutine[JsonValue, JsonValue, None],
]


def target_view(target: InputWriteBackTarget) -> dict[str, JsonValue]:
    match target:
        case SystemStringTarget():
            return {"kind": "system_string", "msg_idx": 0, "content_idx": 0, "block_idx": 0}
        case SystemBlockTextTarget(block_idx=block_idx):
            return {
                "kind": "system_block_text",
                "msg_idx": 0,
                "content_idx": 0,
                "block_idx": block_idx,
            }
        case MessageContentTarget(msg_idx=msg_idx):
            return {"kind": "message_content", "msg_idx": msg_idx, "content_idx": 0, "block_idx": 0}
        case ContentBlockTextTarget(msg_idx=msg_idx, content_idx=content_idx):
            return {
                "kind": "content_block_text",
                "msg_idx": msg_idx,
                "content_idx": content_idx,
                "block_idx": 0,
            }
        case ToolResultStringTarget(msg_idx=msg_idx, content_idx=content_idx):
            return {
                "kind": "tool_result_string",
                "msg_idx": msg_idx,
                "content_idx": content_idx,
                "block_idx": 0,
            }
        case ToolResultBlockTextTarget(
            msg_idx=msg_idx,
            content_idx=content_idx,
            block_idx=block_idx,
        ):
            return {
                "kind": "tool_result_block_text",
                "msg_idx": msg_idx,
                "content_idx": content_idx,
                "block_idx": block_idx,
            }


def text_view(item: ScannedText) -> dict[str, JsonValue]:
    return {
        "text": item.text,
        "target": target_view(item.target),
    }


def extract_answer(scenario: Scenario) -> dict[str, JsonValue]:
    extract = cast(ExtractInput, AnthropicMessagesHandler._extract_input_text_and_images)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L923-L983
    found = extract(
        scenario.message,
        scenario.msg_idx,
        scenario.flags.skip_system_message,
        scenario.flags.skip_tool_message,
        scenario.flags.scan_only_tool_results,
    )
    return {
        "texts": [text_view(item) for item in found.scanned],
        "images": list(found.images),
        "tool_calls": [cast(JsonValue, item.tool_call) for item in found.tool_calls],  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L356-L384
    }


def image_sources_answer(scenario: Scenario) -> dict[str, JsonValue]:
    sources = cast(ImageSources, AnthropicMessagesHandler._image_sources)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L1041-L1064
    return {"images": list(sources(scenario.block))}


def top_level_system_answer(scenario: Scenario) -> dict[str, JsonValue]:
    extract = cast(TopLevelExtract, AnthropicMessagesHandler._extract_top_level_system_text)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L701-L712
    return {"texts": [text_view(item) for item in extract(scenario.hoisted)]}


def structured_messages_answer(scenario: Scenario) -> dict[str, JsonValue]:
    handler = AnthropicMessagesHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L302-L304
    messages = cast(JsonValue, handler.get_structured_messages(scenario.data))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L507-L521
    return {"messages": messages}


def output_apply_answer(scenario: Scenario) -> dict[str, JsonValue]:
    handler = AnthropicMessagesHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L302-L304
    response: dict[str, JsonValue] = {"content": list(scenario.content)}
    mappings: list[tuple[int, int | None]] = [(index, None) for index in scenario.mappings]
    write = cast(WriteOutputResponses, handler._apply_guardrail_responses_to_output)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L1797-L1840
    asyncio.run(write(response, scenario.responses, mappings))
    return {"content": response["content"]}


def post_call_hook_answer(scenario: Scenario) -> dict[str, JsonValue]:
    handler = AnthropicMessagesHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L302-L304
    translated = handler.post_call_hook_response(MODEL_RESPONSE.validate_python(scenario.response))
    return {"response": cast(JsonValue, translated)}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L306-L309


ANSWERS: Final[dict[str, Callable[[Scenario], dict[str, JsonValue]]]] = {
    "extract": extract_answer,
    "image_sources": image_sources_answer,
    "top_level_system": top_level_system_answer,
    "structured_messages": structured_messages_answer,
    "output_apply": output_apply_answer,
    "post_call_hook": post_call_hook_answer,
}


def answers(call: Input) -> list[JsonValue]:
    answer = ANSWERS[call["function"]]
    scenarios = SCENARIOS.validate_python(call["scenarios"])
    output: list[JsonValue] = [answer(scenario) for scenario in scenarios]
    return output


def run(input_value: Input) -> JsonValue:
    litellm.modify_params = True
    litellm.suppress_debug_info = True
    return answers(input_value)
