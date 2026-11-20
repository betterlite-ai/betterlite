import asyncio
from collections.abc import Callable, Coroutine, Mapping, Sequence
from dataclasses import dataclass
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.anthropic.chat.guardrail_translation.handler import (
    AnthropicMessagesHandler,
    ContentBlockTextTarget,
    InputWriteBackTarget,
    MessageContentTarget,
    MessageTextTarget,
    ScannedText,
    SystemBlockTextTarget,
    SystemStringTarget,
    ToolResultBlockTextTarget,
    ToolResultStringTarget,
    ToolUseInputTarget,
    _rewritten_tool_use_input,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L229-L233
    _tool_call_shapes,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L269-L279
    _write_back_message_text,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L205-L223
    _write_back_system_block,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L197-L202
    _write_back_tool_use,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L236-L245
)
from litellm.llms.base_llm.guardrail_translation.utils import UnappliableRequestRewrite
from pydantic import JsonValue


class TargetInput(TypedDict, total=False):
    kind: Literal[
        "system_string",
        "system_block_text",
        "message_content",
        "content_block_text",
        "tool_result_string",
        "tool_result_block_text",
    ]
    msg_idx: int
    content_idx: int
    block_idx: int


class ToolTarget(TypedDict):
    msg_idx: int
    content_idx: int


class ScannedTextItem(TypedDict):
    text: str
    target: TargetInput


class ScannedCallItem(TypedDict):
    tool_call: dict[str, JsonValue]
    tool_target: ToolTarget


class Scenario(TypedDict, total=False):
    system: JsonValue
    block_idx: int
    response: str
    message: dict[str, JsonValue]
    target: TargetInput
    tool_target: ToolTarget
    shape_name: str | None
    arguments: str
    data: dict[str, JsonValue]
    responses: list[str]
    scanned_texts: list[ScannedTextItem]
    scanned_calls: list[ScannedCallItem]
    pre: list[JsonValue]
    returned: list[JsonValue] | None
    guardrail_name: str | None
    messages: list[JsonValue]
    leading: list[JsonValue]
    include_existing: bool
    preserve_system_messages: bool
    compute_hoisted: bool


class Input(TypedDict):
    function: Literal[
        "system_block",
        "message_text",
        "responses_to_input",
        "tool_use",
        "tool_calls",
        "hoisted",
        "system_conversion",
        "fold",
        "defer",
        "structured",
    ]
    scenarios: list[Scenario]


@dataclass(frozen=True, slots=True)
class ScannedCall:
    tool_call: dict[str, JsonValue]
    target: ToolUseInputTarget


class ToolCallShapeView(Protocol):
    name: str | None
    arguments: str


WriteResponses: TypeAlias = Callable[
    [dict[str, JsonValue], Sequence[str], tuple[ScannedText, ...]],
    Coroutine[JsonValue, JsonValue, None],
]
ReadHoisted: TypeAlias = Callable[[Mapping[str, JsonValue]], dict[str, JsonValue] | None]
ConvertSystemRow: TypeAlias = Callable[[Mapping[str, JsonValue]], dict[str, JsonValue] | None]
FoldLeading: TypeAlias = Callable[[dict[str, JsonValue], Sequence[JsonValue], bool], None]
DeferSystems: TypeAlias = Callable[[Sequence[JsonValue]], list[JsonValue]]
WriteStructured: TypeAlias = Callable[
    [dict[str, JsonValue], Sequence[JsonValue], Mapping[str, JsonValue] | None, bool],
    None,
]
ApplyToolCalls: TypeAlias = Callable[
    [
        list[JsonValue],
        Sequence[ScannedCall],
        Sequence[ToolCallShapeView],
        Sequence[JsonValue] | None,
        str | None,
    ],
    None,
]


def message_target(target: TargetInput) -> MessageTextTarget:
    kind = target["kind"]
    if kind == "message_content":
        return MessageContentTarget(msg_idx=target["msg_idx"])
    if kind == "content_block_text":
        return ContentBlockTextTarget(
            msg_idx=target["msg_idx"], content_idx=target["content_idx"]
        )
    if kind == "tool_result_string":
        return ToolResultStringTarget(
            msg_idx=target["msg_idx"], content_idx=target["content_idx"]
        )
    if kind == "tool_result_block_text":
        return ToolResultBlockTextTarget(
            msg_idx=target["msg_idx"],
            content_idx=target["content_idx"],
            block_idx=target["block_idx"],
        )
    raise ValueError(f"not a message target: {kind}")


def input_target(target: TargetInput) -> InputWriteBackTarget:
    kind = target["kind"]
    if kind == "system_string":
        return SystemStringTarget()
    if kind == "system_block_text":
        return SystemBlockTextTarget(block_idx=target["block_idx"])
    return message_target(target)


def tool_use_target(target: ToolTarget) -> ToolUseInputTarget:
    return ToolUseInputTarget(msg_idx=target["msg_idx"], content_idx=target["content_idx"])


def hoisted_reader() -> ReadHoisted:
    handler = AnthropicMessagesHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L302-L304
    return cast(ReadHoisted, handler._hoisted_top_level_system_message)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L686-L699


def system_block_answer(scenario: Scenario) -> dict[str, JsonValue]:
    system = scenario["system"]
    _write_back_system_block(system, scenario["block_idx"], scenario["response"])
    return {"system": system}


def message_text_answer(scenario: Scenario) -> dict[str, JsonValue]:
    message = scenario["message"]
    target = message_target(scenario["target"])
    _write_back_message_text(message, target, scenario["response"])  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L146-L153
    return {"message": message}


def responses_answer(scenario: Scenario) -> dict[str, JsonValue]:
    scanned = tuple(
        ScannedText(text=item["text"], target=input_target(item["target"]))
        for item in scenario["scanned_texts"]
    )
    handler = AnthropicMessagesHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L302-L304
    write = cast(WriteResponses, handler._apply_guardrail_responses_to_input)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L1066-L1092
    asyncio.run(write(scenario["data"], scenario["responses"], scanned))
    return {"data": scenario["data"]}


def tool_use_answer(scenario: Scenario) -> dict[str, JsonValue]:
    message = scenario["message"]
    tool_target = scenario["tool_target"]
    target = ToolUseInputTarget(
        msg_idx=tool_target["msg_idx"], content_idx=tool_target["content_idx"]
    )
    call_function = {"name": scenario["shape_name"], "arguments": scenario["arguments"]}
    shape = _tool_call_shapes([{"function": call_function}])[0]
    rewritten = _rewritten_tool_use_input(scenario["arguments"])
    if rewritten is None:
        return {"message": message, "input": None}
    _write_back_tool_use(message, target, shape, rewritten)  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L146-L153
    return {"message": message, "input": cast(dict[str, JsonValue], rewritten)}


def tool_calls_answer(scenario: Scenario) -> dict[str, JsonValue]:
    messages = scenario["messages"]
    scanned = tuple(
        ScannedCall(
            tool_call=item["tool_call"],
            target=ToolUseInputTarget(
                msg_idx=item["tool_target"]["msg_idx"],
                content_idx=item["tool_target"]["content_idx"],
            ),
        )
        for item in scenario["scanned_calls"]
    )
    pre = _tool_call_shapes(scenario["pre"])
    apply = cast(ApplyToolCalls, AnthropicMessagesHandler._apply_guardrail_tool_calls_to_input)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L1094-L1118
    outcome = "ok"
    try:
        apply(messages, scanned, pre, scenario["returned"], scenario["guardrail_name"])
    except UnappliableRequestRewrite:
        outcome = "unappliable"
    return {"messages": messages, "outcome": outcome}


def hoisted_answer(scenario: Scenario) -> dict[str, JsonValue]:
    message = hoisted_reader()(scenario["data"])
    return {"message": message}


def system_conversion_answer(scenario: Scenario) -> dict[str, JsonValue]:
    convert = cast(ConvertSystemRow, AnthropicMessagesHandler._openai_system_message_to_anthropic)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L714-L739
    row = convert(scenario["message"])
    return {"row": row}


def fold_answer(scenario: Scenario) -> dict[str, JsonValue]:
    fold = cast(FoldLeading, AnthropicMessagesHandler._fold_leading_systems_into_top_level)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L741-L772
    fold(scenario["data"], scenario["leading"], scenario["include_existing"])
    return {"data": scenario["data"]}


def defer_answer(scenario: Scenario) -> dict[str, JsonValue]:
    defer = cast(DeferSystems, AnthropicMessagesHandler._defer_systems_inside_tool_exchanges)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L790-L820
    return {"messages": defer(scenario["messages"])}


def structured_answer(scenario: Scenario) -> dict[str, JsonValue]:
    data = scenario["data"]
    hoisted = hoisted_reader()(data) if scenario["compute_hoisted"] else None
    write = cast(WriteStructured, AnthropicMessagesHandler._write_back_structured_messages)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L822-L891
    write(data, scenario["messages"], hoisted, scenario["preserve_system_messages"])
    return {"data": data}


ANSWERS: Final[dict[str, Callable[[Scenario], dict[str, JsonValue]]]] = {
    "system_block": system_block_answer,
    "message_text": message_text_answer,
    "responses_to_input": responses_answer,
    "tool_use": tool_use_answer,
    "tool_calls": tool_calls_answer,
    "hoisted": hoisted_answer,
    "system_conversion": system_conversion_answer,
    "fold": fold_answer,
    "defer": defer_answer,
    "structured": structured_answer,
}


def answers(call: Input) -> list[JsonValue]:
    answer = ANSWERS[call["function"]]
    output: list[JsonValue] = [answer(scenario) for scenario in call["scenarios"]]
    return output


def run(input_value: Input) -> JsonValue:
    litellm.modify_params = True
    litellm.suppress_debug_info = True
    return answers(input_value)
