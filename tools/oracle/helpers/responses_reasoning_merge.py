from typing import Any, TypeAlias, cast

from litellm.responses.litellm_completion_transformation.transformation import (
    LiteLLMCompletionResponsesConfig,
)

# pyright: reportAny=false

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

def responses_reasoning_merge(input_value: dict[str, Json]) -> Json:
    mode = input_value["mode"]
    if mode == "reasoning":
        messages = cast(Any, input_value["messages"])  # pyright: ignore[reportAny]
        result = LiteLLMCompletionResponsesConfig._merge_reasoning_only_assistant_messages(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            messages  # pyright: ignore[reportAny]
        )
        return cast(Json, result)
    if mode == "trailing":
        messages = cast(Any, input_value["messages"])  # pyright: ignore[reportAny]
        new_messages = cast(Any, input_value.get("new_messages", []))  # pyright: ignore[reportAny]
        hosted_search = cast(bool, input_value.get("hosted_search", False))
        return cast(Json, LiteLLMCompletionResponsesConfig._merged_trailing_assistant_message(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAny]
            messages=messages, chat_completion_messages=new_messages, hosted_search=hosted_search
        ))
    raise ValueError("unknown merge mode")
