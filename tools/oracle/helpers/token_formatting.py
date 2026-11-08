from typing import TypeAlias

from litellm.litellm_core_utils import token_counter
from litellm.litellm_core_utils.token_counter import (
    _fix_model_name,  # pyright: ignore[reportPrivateUsage]
    _format_function_definitions,  # pyright: ignore[reportPrivateUsage]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def model_value(input_value: dict[str, Json]) -> str:
    value = input_value.get("model")
    if not isinstance(value, str):
        raise ValueError("model must be a string")
    return value


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "encoding_name":
        return token_counter.openai_tokenizer_encoding_name(model_value(input_value))
    if function == "fix_model_name":
        return _fix_model_name(model_value(input_value))
    if function == "legacy_accounting":
        return token_counter.uses_legacy_message_accounting(model_value(input_value))
    if function == "format_tools":
        tools = input_value.get("tools")
        if not isinstance(tools, list):
            raise ValueError("tools must be a list")
        return _format_function_definitions(tools)
    raise ValueError(f"unknown token formatting function: {function}")
