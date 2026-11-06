from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.litellm_core_utils.prompt_templates import common_utils
from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _get_format_from_file_id(input_value: dict[str, Json]) -> Json:
    file_id = cast(str | None, input_value.get("file_id"))
    return cast(Json, common_utils.get_format_from_file_id(file_id))


def _update_messages_with_model_file_ids(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value["messages"])
    model_id = cast(str | None, input_value.get("model_id"))
    mapping = cast(dict[str, dict[str, str]], input_value.get("mapping", {}))
    result = common_utils.update_messages_with_model_file_ids(messages, model_id, mapping)
    return cast(Json, result)


def _update_responses_input_with_model_file_ids(input_value: dict[str, Json]) -> Json:
    input_value_ = input_value.get("input")
    model_id = cast(str | None, input_value.get("model_id"))
    mapping = cast(dict[str, dict[str, str]], input_value.get("mapping", {}))
    result = common_utils.update_responses_input_with_model_file_ids(
        input_value_, model_id, mapping
    )
    return cast(Json, result)


def _decode_vector_store_ids_in_tools(input_value: dict[str, Json]) -> Json:
    tools = cast(list[dict[str, object]] | None, input_value.get("tools"))
    result = common_utils._decode_vector_store_ids_in_tools(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        tools
    )
    return cast(Json, result)


def _update_responses_tools_with_model_file_ids(input_value: dict[str, Json]) -> Json:
    tools = cast(list[dict[str, object]] | None, input_value.get("tools"))
    model_id = cast(str | None, input_value.get("model_id"))
    mapping = cast(dict[str, dict[str, str]], input_value.get("mapping", {}))
    result = common_utils.update_responses_tools_with_model_file_ids(tools, model_id, mapping)
    return cast(Json, result)


def _get_file_ids_from_messages(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value["messages"])
    return cast(Json, common_utils.get_file_ids_from_messages(messages))


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "get_format_from_file_id": _get_format_from_file_id,
    "update_messages_with_model_file_ids": _update_messages_with_model_file_ids,
    "update_responses_input_with_model_file_ids": _update_responses_input_with_model_file_ids,
    "decode_vector_store_ids_in_tools": _decode_vector_store_ids_in_tools,
    "update_responses_tools_with_model_file_ids": _update_responses_tools_with_model_file_ids,
    "get_file_ids_from_messages": _get_file_ids_from_messages,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["function"])
    if operation not in HANDLERS:
        raise ValueError(f"unsupported function {operation}")
    return HANDLERS[operation](input_value)
