from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.litellm_core_utils.prompt_templates import factory
from litellm.litellm_core_utils.prompt_templates.factory import (
    convert_to_gemini_tool_call_result,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.types.llms.openai import (
    AllMessageValues,
    ChatCompletionFunctionMessage,
    ChatCompletionToolMessage,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _MappedConvert:
    def __init__(self, mapping: dict[str, str]) -> None:
        self.mapping = mapping

    def __call__(self, url: str) -> str:
        return self.mapping[url]


def run(input_value: dict[str, Json]) -> Json:
    message = cast(
        "ChatCompletionToolMessage | ChatCompletionFunctionMessage",
        cast(AllMessageValues, input_value["message"]),
    )
    last_message = cast("dict[str, Json] | None", input_value.get("last_message_with_tool_calls"))
    forward_function_call_id = cast(bool, input_value.get("forward_function_call_id", False))
    mapping = cast("dict[str, str]", input_value.get("data_urls", {}))
    with patch.object(factory, "convert_url_to_base64", _MappedConvert(mapping)):
        return cast(
            Json,
            convert_to_gemini_tool_call_result(
                message,
                last_message,
                forward_function_call_id=forward_function_call_id,
            ),
        )
