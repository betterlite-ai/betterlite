import copy
from typing import TypeAlias, cast

from litellm.llms.anthropic import common_utils

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "advisor":
        result = _advisor(input_value)
    elif function == "invalid_thinking_error":
        result = _invalid_thinking_error(input_value)
    elif function == "strip_encrypted":
        result = _strip_encrypted(input_value)
    elif function == "empty_predicates":
        result = _empty_predicates(input_value)
    elif function == "strip_empty_content":
        result = _strip_empty_content(input_value)
    elif function == "tool_use_ids":
        result = _tool_use_ids(input_value)
    elif function == "provider_fields":
        result = _provider_fields(input_value)
    elif function == "cache_control":
        result = _cache_control(input_value)
    else:
        raise ValueError(f"unknown function {function}")
    return result


def _advisor(input_value: dict[str, Json]) -> Json:
    messages = copy.deepcopy(cast("list[Json]", input_value["messages"]))
    return cast(
        Json,
        common_utils.strip_advisor_blocks_from_messages(  # pyright: ignore[reportUnknownMemberType]
            cast("list[object]", messages),
            cast(bool, input_value.get("replace_with_text", False)),
        ),
    )


def _invalid_thinking_error(input_value: dict[str, Json]) -> Json:
    texts = cast("list[str]", input_value["texts"])
    matches = [common_utils.is_anthropic_invalid_thinking_block_error(text) for text in texts]
    requests = copy.deepcopy(cast("list[dict[str, object]]", input_value["requests"]))
    for request in requests:
        common_utils.strip_thinking_blocks_from_anthropic_messages_request_dict(request)
    return cast(Json, {"matches": matches, "repaired": requests})


def _strip_encrypted(input_value: dict[str, Json]) -> Json:
    messages = copy.deepcopy(cast("list[dict[str, object]]", input_value["messages"]))
    return cast(
        Json,
        common_utils.strip_encrypted_reasoning_blocks_from_anthropic_messages(  # pyright: ignore[reportUnknownMemberType]
            messages,
        ),
    )


def _empty_predicates(input_value: dict[str, Json]) -> Json:
    blocks = cast("list[object]", input_value["blocks"])
    thinking = [common_utils.is_empty_thinking_block(block) for block in blocks]
    unsigned = [common_utils.is_empty_unsigned_thinking_block(block) for block in blocks]
    text = [
        common_utils._is_empty_text_block(block)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        for block in blocks
    ]
    return cast(Json, {"thinking": thinking, "unsigned": unsigned, "text": text})


def _strip_empty_content(input_value: dict[str, Json]) -> Json:
    messages = copy.deepcopy(cast("list[object]", input_value["messages"]))
    stripped = common_utils.strip_empty_content_blocks_from_anthropic_messages(messages)
    return cast(Json, stripped)


def _tool_use_ids(input_value: dict[str, Json]) -> Json:
    messages = copy.deepcopy(cast("list[object]", input_value["messages"]))
    sanitized = common_utils.sanitize_tool_use_ids_in_anthropic_messages(messages)
    return cast(Json, sanitized)


def _provider_fields(input_value: dict[str, Json]) -> Json:
    messages = copy.deepcopy(cast("list[object]", input_value["messages"]))
    stripped = common_utils.strip_provider_specific_fields_from_anthropic_messages(messages)
    return cast(Json, stripped)


def _cache_control(input_value: dict[str, Json]) -> Json:
    payload = copy.deepcopy(cast("dict[str, object]", input_value["payload"]))
    portable = common_utils.normalize_cache_control_in_anthropic_payload(payload)
    return cast(Json, portable)
