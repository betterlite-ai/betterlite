from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.litellm_core_utils.prompt_templates import common_utils
from litellm.types.llms.openai import AllMessageValues, ChatCompletionFileObject

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _strip_litellm_internal_message_fields(input_value: dict[str, Json]) -> Json:
    message = cast(AllMessageValues, input_value["message"])
    return cast(Json, common_utils.strip_litellm_internal_message_fields(message))


def _filter_value_from_dict(input_value: dict[str, Json]) -> Json:
    dictionary = cast(dict[str, Json], input_value["dictionary"])
    key = cast(str, input_value["key"])
    depth = cast(int, input_value.get("depth", 0))
    result: dict[str, Json] = common_utils.filter_value_from_dict(  # pyright: ignore[reportUnknownMemberType, reportAny]
        dictionary, key, depth
    )
    return cast(Json, result)


def _migrate_file_to_image_url(input_value: dict[str, Json]) -> Json:
    part = cast(ChatCompletionFileObject, input_value["part"])
    return cast(Json, common_utils.migrate_file_to_image_url(part))


def _get_last_user_message(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value["messages"])
    return common_utils.get_last_user_message(messages)


def _set_last_user_message(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value["messages"])
    content = cast(str, input_value["content"])
    return cast(Json, common_utils.set_last_user_message(messages, content))


def _add_system_prompt_to_messages(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value["messages"])
    system_prompt = cast(str, input_value["system_prompt"])
    merge_with_first_system = cast(bool, input_value.get("merge_with_first_system", False))
    result = common_utils.add_system_prompt_to_messages(
        messages, system_prompt, merge_with_first_system
    )
    return cast(Json, result)


def _convert_prefix_message_to_non_prefix_messages(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value["messages"])
    return cast(Json, common_utils.convert_prefix_message_to_non_prefix_messages(messages))


def _extract_images_from_message(input_value: dict[str, Json]) -> Json:
    message = cast(AllMessageValues, input_value["message"])
    return cast(Json, common_utils.extract_images_from_message(message))


def _hoist_images_from_tool_messages(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value["messages"])
    return cast(Json, common_utils.hoist_images_from_tool_messages(messages))


def _drop_tool_reference_parts_from_tool_messages(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value["messages"])
    return cast(Json, common_utils.drop_tool_reference_parts_from_tool_messages(messages))


def _system_messages_first(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value["messages"])
    return cast(Json, common_utils.system_messages_first(messages))


def _merge_consecutive_system_messages(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value["messages"])
    return cast(Json, common_utils.merge_consecutive_system_messages(messages))


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "strip_litellm_internal_message_fields": _strip_litellm_internal_message_fields,
    "filter_value_from_dict": _filter_value_from_dict,
    "migrate_file_to_image_url": _migrate_file_to_image_url,
    "get_last_user_message": _get_last_user_message,
    "set_last_user_message": _set_last_user_message,
    "add_system_prompt_to_messages": _add_system_prompt_to_messages,
    "convert_prefix_message_to_non_prefix_messages": _convert_prefix_message_to_non_prefix_messages,
    "extract_images_from_message": _extract_images_from_message,
    "hoist_images_from_tool_messages": _hoist_images_from_tool_messages,
    "drop_tool_reference_parts_from_tool_messages": _drop_tool_reference_parts_from_tool_messages,
    "system_messages_first": _system_messages_first,
    "merge_consecutive_system_messages": _merge_consecutive_system_messages,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    if operation not in HANDLERS:
        raise ValueError(f"unsupported operation {operation}")
    return HANDLERS[operation](input_value)
