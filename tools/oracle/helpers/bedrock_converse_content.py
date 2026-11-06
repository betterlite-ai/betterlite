from collections.abc import Callable
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch
from uuid import UUID

import litellm
from litellm.llms.bedrock.chat.converse_transformation import AmazonConverseConfig
from litellm.types.utils import Message

if TYPE_CHECKING:
    from litellm.types.llms.bedrock import CitationsContentBlock, ContentBlock, ToolBlock
    from litellm.types.llms.openai import ChatCompletionToolCallChunk

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

FIXED_ID = UUID(int=0)


def object_field(fields: Object, name: str) -> Object:
    value = fields.get(name)
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def optional_list(fields: Object, name: str) -> list[Json] | None:
    value = fields.get(name)
    if value is None:
        return None
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def optional_string(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def set_tool_name_mappings(fields: Object) -> None:
    mappings = object_field(fields, "mappings")
    litellm.bedrock_tool_name_mappings.cache_dict.update(  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
        cast(dict[str, str], mappings)
    )


def reverse_tool_names(fields: Object) -> Json:
    tools = optional_list(fields, "tools")
    names = AmazonConverseConfig().get_tool_call_names(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        cast("list[ToolBlock] | None", tools)  # pyright: ignore[reportArgumentType]
    )
    return cast(Json, sorted(names))


def json_mode_filter(fields: Object) -> Json:
    message = object_field(fields, "message")
    tools = optional_list(fields, "tools")
    json_mode = fields.get("json_mode")
    if json_mode is not None and not isinstance(json_mode, bool):
        raise ValueError("json_mode must be a boolean or null")
    filtered = AmazonConverseConfig._filter_json_mode_tools(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        json_mode,
        cast("list[ChatCompletionToolCallChunk]", tools),  # pyright: ignore[reportArgumentType]
        message,  # pyright: ignore[reportArgumentType]
    )
    return {"tools": cast(Json, filtered), "content": message.get("content")}


def wrapped_properties(fields: Object) -> Json:
    value = optional_string(fields, "value")
    if value is None:
        raise ValueError("value must be a string")
    return AmazonConverseConfig._unwrap_bedrock_properties(value)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def translate(fields: Object) -> tuple[Json, Json, Json, Json]:
    set_tool_name_mappings(fields)
    blocks = optional_list(fields, "content_blocks")
    if blocks is None:
        raise ValueError("content_blocks must be a list")
    content, tools, reasoning, citations = AmazonConverseConfig()._translate_message_content(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
        cast("list[ContentBlock]", blocks)  # pyright: ignore[reportArgumentType]
    )
    return (
        cast(Json, content),
        cast(Json, tools),
        cast(Json, reasoning),
        cast(Json, citations),
    )


def text_with_tool_use(fields: Object) -> Json:
    content, tools, reasoning, citations = translate(fields)
    return {
        "content": content,
        "tools": tools,
        "reasoning_content_blocks": reasoning,
        "citations_content_blocks": citations,
    }


def empty_tool_input(fields: Object) -> Json:
    _content, tools, _reasoning, _citations = translate(fields)
    return {"tools": tools}


def content_function_recovery(fields: Object) -> Json:
    message_fields = object_field(fields, "message")
    message = Message(
        content=optional_string(message_fields, "content"),
        role="assistant",
        tool_calls=optional_list(message_fields, "tool_calls"),  # pyright: ignore[reportArgumentType]
    )
    tools = optional_list(fields, "tools")
    initial_finish_reason = optional_string(fields, "initial_finish_reason")
    with patch("litellm.types.utils.uuid.uuid4", return_value=FIXED_ID):
        returned, finish_reason = AmazonConverseConfig().apply_tool_call_transformation_if_needed(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            message,
            cast("list[ToolBlock] | None", tools),  # pyright: ignore[reportArgumentType]
            initial_finish_reason,
        )
    tool_calls: Json = None
    if returned.tool_calls is not None:
        tool_calls = cast(Json, [call.model_dump() for call in returned.tool_calls])  # pyright: ignore[reportAny, reportUnknownMemberType, reportUnknownVariableType]
    return {
        "message": {"content": returned.content, "tool_calls": tool_calls},
        "finish_reason": finish_reason,
    }


def citation_search_locations(fields: Object) -> Json:
    blocks = fields.get("citations_content_blocks")
    if blocks is not None and not isinstance(blocks, list):
        raise ValueError("citations_content_blocks must be a list")
    text, annotations = AmazonConverseConfig._transform_citations_to_annotations(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        cast("list[CitationsContentBlock] | None", blocks)  # pyright: ignore[reportArgumentType]
    )
    return {"text": text, "annotations": cast(Json, annotations)}


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "reverse_tool_names": reverse_tool_names,
    "json_mode_filter": json_mode_filter,
    "wrapped_properties": wrapped_properties,
    "text_with_tool_use": text_with_tool_use,
    "empty_tool_input": empty_tool_input,
    "content_function_recovery": content_function_recovery,
    "citation_search_locations": citation_search_locations,
}


def run(input_value: dict[str, Json]) -> Json:
    [(operation, request)] = input_value.items()
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unsupported operation {operation}")
    return handler(cast(Object, request))
