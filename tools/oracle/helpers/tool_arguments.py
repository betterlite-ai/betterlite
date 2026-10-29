from collections.abc import Callable
from typing import TYPE_CHECKING, TypeAlias, cast

from litellm.litellm_core_utils.prompt_templates.common_utils import (
    check_is_function_call,
    convert_custom_tool_format_to_chat_shape,
    convert_custom_tool_format_to_responses_shape,
    get_tool_call_names,
    is_function_call,  # pyright: ignore[reportUnknownVariableType]
    parse_tool_call_arguments,
    salvage_concatenated_tool_arguments,
    split_concatenated_json_objects,
    with_prompt_cache_breakpoint,
)

if TYPE_CHECKING:
    from litellm.types.llms.openai import ChatCompletionToolParam

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _LoggingObj:
    def __init__(self, optional_params: dict[str, Json] | None) -> None:
        if optional_params is not None:
            self.optional_params = optional_params


def _parse_tool_call_arguments(input_value: dict[str, Json]) -> Json:
    arguments = cast(str, input_value["arguments"])
    tool_name = cast(str | None, input_value.get("tool_name"))
    context = cast(str | None, input_value.get("context"))
    return cast(Json, parse_tool_call_arguments(arguments, tool_name, context))


def _split_concatenated_json_objects(input_value: dict[str, Json]) -> Json:
    raw = cast(str, input_value["raw"])
    return cast(Json, split_concatenated_json_objects(raw))


def _salvage_concatenated_tool_arguments(input_value: dict[str, Json]) -> Json:
    raw = cast(str, input_value["raw"])
    return cast(Json, list(salvage_concatenated_tool_arguments(raw)))


def _get_tool_call_names(input_value: dict[str, Json]) -> Json:
    tools = cast(list[dict[str, Json]], input_value["tools"])
    return cast(Json, get_tool_call_names(cast("list[ChatCompletionToolParam]", tools)))


def _is_function_call(input_value: dict[str, Json]) -> Json:
    optional_params = cast(dict[str, Json], input_value["optional_params"])
    return is_function_call(optional_params)


def _check_is_function_call(input_value: dict[str, Json]) -> Json:
    optional_params = cast(dict[str, Json] | None, input_value.get("optional_params"))
    logging_obj = _LoggingObj(optional_params)
    return check_is_function_call(logging_obj)  # pyright: ignore[reportArgumentType]


def _convert_custom_tool_format_to_chat_shape(input_value: dict[str, Json]) -> Json:
    format_obj = cast(dict[str, Json], input_value["format_obj"])
    return cast(Json, convert_custom_tool_format_to_chat_shape(format_obj))


def _convert_custom_tool_format_to_responses_shape(input_value: dict[str, Json]) -> Json:
    format_obj = cast(dict[str, Json], input_value["format_obj"])
    return cast(Json, convert_custom_tool_format_to_responses_shape(format_obj))


def _with_prompt_cache_breakpoint(input_value: dict[str, Json]) -> Json:
    target = cast(dict[str, Json], input_value["target"])
    marker = input_value.get("marker")
    return cast(Json, with_prompt_cache_breakpoint(target, marker))


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "parse_tool_call_arguments": _parse_tool_call_arguments,
    "split_concatenated_json_objects": _split_concatenated_json_objects,
    "salvage_concatenated_tool_arguments": _salvage_concatenated_tool_arguments,
    "get_tool_call_names": _get_tool_call_names,
    "is_function_call": _is_function_call,
    "check_is_function_call": _check_is_function_call,
    "convert_custom_tool_format_to_chat_shape": _convert_custom_tool_format_to_chat_shape,
    "convert_custom_tool_format_to_responses_shape": _convert_custom_tool_format_to_responses_shape,
    "with_prompt_cache_breakpoint": _with_prompt_cache_breakpoint,
}


def run(input_value: dict[str, Json]) -> Json:
    mode = cast(str, input_value["mode"])
    if mode not in HANDLERS:
        raise ValueError(f"unsupported mode {mode}")
    return HANDLERS[mode](input_value)
