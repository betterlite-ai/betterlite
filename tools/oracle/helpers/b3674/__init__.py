from collections.abc import Callable, Sequence
from typing import TypeAlias, TypeVar

import litellm
from pydantic import JsonValue

from .structured_write_back import (
    DeferRequest,
    FoldRequest,
    HoistedRequest,
    StructuredRequest,
    SystemConversionRequest,
    defer_answer,
    fold_answer,
    hoisted_answer,
    structured_answer,
    system_conversion_answer,
)
from .write_back import (
    MessageTextRequest,
    ResponsesToInputRequest,
    SystemBlockRequest,
    ToolCallsRequest,
    ToolUseRequest,
    message_text_answer,
    responses_answer,
    system_block_answer,
    tool_calls_answer,
    tool_use_answer,
)

Input: TypeAlias = (
    DeferRequest
    | FoldRequest
    | HoistedRequest
    | MessageTextRequest
    | ResponsesToInputRequest
    | StructuredRequest
    | SystemBlockRequest
    | SystemConversionRequest
    | ToolCallsRequest
    | ToolUseRequest
)

Case = TypeVar("Case")


def answers(
    scenarios: Sequence[Case], answer: Callable[[Case], dict[str, JsonValue]]
) -> list[JsonValue]:
    output: list[JsonValue] = []
    for scenario in scenarios:
        output.append(answer(scenario))
    return output


def run(input_value: Input) -> JsonValue:
    litellm.modify_params = True
    litellm.suppress_debug_info = True
    result: JsonValue
    if input_value["function"] == "system_block":
        result = answers(input_value["scenarios"], system_block_answer)
    elif input_value["function"] == "message_text":
        result = answers(input_value["scenarios"], message_text_answer)
    elif input_value["function"] == "responses_to_input":
        result = answers(input_value["scenarios"], responses_answer)
    elif input_value["function"] == "tool_use":
        result = answers(input_value["scenarios"], tool_use_answer)
    elif input_value["function"] == "tool_calls":
        result = answers(input_value["scenarios"], tool_calls_answer)
    elif input_value["function"] == "hoisted":
        result = answers(input_value["scenarios"], hoisted_answer)
    elif input_value["function"] == "system_conversion":
        result = answers(input_value["scenarios"], system_conversion_answer)
    elif input_value["function"] == "fold":
        result = answers(input_value["scenarios"], fold_answer)
    elif input_value["function"] == "defer":
        result = answers(input_value["scenarios"], defer_answer)
    else:
        result = answers(input_value["scenarios"], structured_answer)
    return result
