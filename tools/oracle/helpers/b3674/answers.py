import asyncio
from typing import Final, cast

import litellm
from litellm.llms.anthropic.chat.guardrail_translation.handler import (
    AnthropicMessagesHandler,
    ContentBlockTextTarget,
    InputWriteBackTarget,
    MessageContentTarget,
    MessageTextTarget,
    ScannedText,
    ScannedToolCall,
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
from pydantic import JsonValue, TypeAdapter

from .inputs import (
    DeferScenario,
    Deferrer,
    FoldScenario,
    HoistedReader,
    HoistedScenario,
    Input,
    InputTargetInput,
    MessageTargetInput,
    MessageTextScenario,
    ResponsesScenario,
    ResponsesWriter,
    StructuredScenario,
    SystemBlockScenario,
    SystemConversionScenario,
    SystemRowConverter,
    ToolCallsScenario,
    ToolUseScenario,
)

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


def message_target(target: MessageTargetInput) -> MessageTextTarget:
    if target["kind"] == "message_content":
        return MessageContentTarget(msg_idx=target["msg_idx"])
    if target["kind"] == "content_block_text":
        return ContentBlockTextTarget(
            msg_idx=target["msg_idx"], content_idx=target["content_idx"]
        )
    if target["kind"] == "tool_result_string":
        return ToolResultStringTarget(
            msg_idx=target["msg_idx"], content_idx=target["content_idx"]
        )
    return ToolResultBlockTextTarget(
        msg_idx=target["msg_idx"],
        content_idx=target["content_idx"],
        block_idx=target["block_idx"],
    )


def input_target(target: InputTargetInput) -> InputWriteBackTarget:
    if target["kind"] == "system_string":
        return SystemStringTarget()
    if target["kind"] == "system_block_text":
        return SystemBlockTextTarget(block_idx=target["block_idx"])
    if target["kind"] == "message_content":
        return MessageContentTarget(msg_idx=target["msg_idx"])
    if target["kind"] == "content_block_text":
        return ContentBlockTextTarget(
            msg_idx=target["msg_idx"], content_idx=target["content_idx"]
        )
    if target["kind"] == "tool_result_string":
        return ToolResultStringTarget(
            msg_idx=target["msg_idx"], content_idx=target["content_idx"]
        )
    return ToolResultBlockTextTarget(
        msg_idx=target["msg_idx"],
        content_idx=target["content_idx"],
        block_idx=target["block_idx"],
    )


def system_block_answer(scenario: SystemBlockScenario) -> JsonValue:
    system = scenario["system"]
    _write_back_system_block(system, scenario["block_idx"], scenario["response"])
    return {"system": system}


def message_text_answer(scenario: MessageTextScenario) -> JsonValue:
    target = message_target(scenario["target"])
    _write_back_message_text(
        scenario["message"],  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L146-L153
        target,
        scenario["response"],
    )
    return {"message": scenario["message"]}


def responses_answer(scenario: ResponsesScenario) -> JsonValue:
    scanned = tuple(
        ScannedText(text=item["text"], target=input_target(item["target"]))
        for item in scenario["scanned"]
    )
    handler = AnthropicMessagesHandler()
    write = cast(ResponsesWriter, handler._apply_guardrail_responses_to_input)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L1066-L1092
    asyncio.run(write(scenario["data"], scenario["responses"], scanned))
    return {"data": scenario["data"]}


def tool_use_answer(scenario: ToolUseScenario) -> JsonValue:
    target = ToolUseInputTarget(
        msg_idx=scenario["target"]["msg_idx"],
        content_idx=scenario["target"]["content_idx"],
    )
    shape = _tool_call_shapes(
        [
            {
                "function": {
                    "name": scenario["shape_name"],
                    "arguments": scenario["arguments"],
                }
            }
        ]
    )[0]
    rewritten = _rewritten_tool_use_input(scenario["arguments"])
    if rewritten is None:
        return {"message": scenario["message"], "input": None}
    _write_back_tool_use(
        scenario["message"],  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L146-L153
        target,
        shape,
        rewritten,
    )
    return {"message": scenario["message"], "input": JSON.validate_python(rewritten)}


def tool_calls_answer(scenario: ToolCallsScenario) -> JsonValue:
    scanned = tuple(
        ScannedToolCall(
            tool_call=item["tool_call"],
            target=ToolUseInputTarget(
                msg_idx=item["target"]["msg_idx"],
                content_idx=item["target"]["content_idx"],
            ),
        )
        for item in scenario["scanned"]
    )
    pre = _tool_call_shapes(scenario["pre"])
    apply = AnthropicMessagesHandler._apply_guardrail_tool_calls_to_input  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L1094-L1118
    outcome: str
    try:
        apply(
            messages=scenario["messages"],  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L1096
            scanned_tool_calls=scanned,
            pre_guardrail_tool_calls=pre,
            returned_tool_calls=scenario["returned"],
            guardrail_name=scenario["guardrail_name"],
        )
    except UnappliableRequestRewrite:
        outcome = "unappliable"
    else:
        outcome = "ok"
    return {"messages": scenario["messages"], "outcome": outcome}


def hoisted_answer(scenario: HoistedScenario) -> JsonValue:
    handler = AnthropicMessagesHandler()
    read = cast(HoistedReader, handler._hoisted_top_level_system_message)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L686-L699
    return {"message": read(scenario["data"])}


def system_conversion_answer(scenario: SystemConversionScenario) -> JsonValue:
    convert = cast(SystemRowConverter, AnthropicMessagesHandler._openai_system_message_to_anthropic)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L714-L739
    return {"row": convert(scenario["message"])}


def fold_answer(scenario: FoldScenario) -> JsonValue:
    fold = AnthropicMessagesHandler._fold_leading_systems_into_top_level  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L741-L772
    fold(
        scenario["data"],  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L742-L746
        scenario["leading"],
        scenario["include_existing"],
    )
    return {"data": scenario["data"]}


def defer_answer(scenario: DeferScenario) -> JsonValue:
    defer = cast(Deferrer, AnthropicMessagesHandler._defer_systems_inside_tool_exchanges)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L790-L820
    return {"messages": defer(scenario["messages"])}


def structured_answer(scenario: StructuredScenario) -> JsonValue:
    data = scenario["data"]
    hoisted: dict[str, JsonValue] | None = None
    if scenario["compute_hoisted"]:
        handler = AnthropicMessagesHandler()
        read = cast(HoistedReader, handler._hoisted_top_level_system_message)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L686-L699
        hoisted = read(data)
    write_structured = AnthropicMessagesHandler._write_back_structured_messages  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L822-L891
    write_structured(data, scenario["messages"], hoisted, scenario["preserve_system_messages"])
    return {"data": data}


def answers(call: Input) -> list[JsonValue]:
    answer: list[JsonValue]
    if call["function"] == "system_block":
        answer = [system_block_answer(scenario) for scenario in call["scenarios"]]
    elif call["function"] == "message_text":
        answer = [message_text_answer(scenario) for scenario in call["scenarios"]]
    elif call["function"] == "responses_to_input":
        answer = [responses_answer(scenario) for scenario in call["scenarios"]]
    elif call["function"] == "tool_use":
        answer = [tool_use_answer(scenario) for scenario in call["scenarios"]]
    elif call["function"] == "tool_calls":
        answer = [tool_calls_answer(scenario) for scenario in call["scenarios"]]
    elif call["function"] == "hoisted":
        answer = [hoisted_answer(scenario) for scenario in call["scenarios"]]
    elif call["function"] == "system_conversion":
        answer = [system_conversion_answer(scenario) for scenario in call["scenarios"]]
    elif call["function"] == "fold":
        answer = [fold_answer(scenario) for scenario in call["scenarios"]]
    elif call["function"] == "defer":
        answer = [defer_answer(scenario) for scenario in call["scenarios"]]
    else:
        answer = [structured_answer(scenario) for scenario in call["scenarios"]]
    return answer


def run(input_value: Input) -> JsonValue:
    litellm.modify_params = True
    litellm.suppress_debug_info = True
    return JSON.validate_python(answers(input_value))
