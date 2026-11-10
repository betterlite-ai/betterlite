import copy
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.litellm_core_utils.prompt_templates.factory import (
    _bedrock_converse_messages_pt,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
)

if TYPE_CHECKING:
    from litellm.types.llms.openai import (
        AllMessageValues,
        ChatCompletionAssistantMessage,
        ChatCompletionUserMessage,
    )

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _convert(input_value: dict[str, Json]) -> Json:
    litellm.modify_params = cast(bool, input_value.get("modify_params", False))
    messages = copy.deepcopy(cast("list[AllMessageValues]", input_value["messages"]))
    return cast(
        Json,
        _bedrock_converse_messages_pt(
            messages=messages,
            model=cast(str, input_value.get("model")),
            llm_provider="bedrock",
            user_continue_message=cast(
                "ChatCompletionUserMessage | None",
                input_value.get("user_continue_message"),
            ),
            assistant_continue_message=cast(
                "str | ChatCompletionAssistantMessage | None",
                input_value.get("assistant_continue_message"),
            ),
        ),
    )


def run(input_value: dict[str, Json]) -> Json:
    match input_value.get("function"):
        case "convert":
            return _convert(input_value)
        case "convert_twice":
            return [_convert(input_value), _convert(input_value)]
        case other:
            raise ValueError(f"unknown bedrock_processor function: {other!r}")
