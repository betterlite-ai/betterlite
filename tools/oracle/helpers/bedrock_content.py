from collections.abc import Callable
from typing import TypeAlias, cast

import litellm
from litellm.litellm_core_utils.prompt_templates.factory import (
    _deduplicate_bedrock_content_blocks,  # pyright: ignore[reportPrivateUsage]
    _deduplicate_bedrock_tool_content,  # pyright: ignore[reportPrivateUsage]
    _ensure_document_messages_have_text,  # pyright: ignore[reportPrivateUsage]
    _insert_assistant_continue_message,  # pyright: ignore[reportPrivateUsage]
    _rename_duplicate_bedrock_document_names,  # pyright: ignore[reportPrivateUsage]
    _sort_bedrock_assistant_content_blocks,  # pyright: ignore[reportPrivateUsage]
    _with_text_when_document_only,  # pyright: ignore[reportPrivateUsage]
    get_assistant_message_block_or_continue_message,
    get_user_message_block_or_continue_message,
    process_empty_text_blocks,
    return_assistant_continue_message,
    skip_empty_text_blocks,
)
from litellm.types.llms.bedrock import (
    ContentBlock as BedrockContentBlock,
)
from litellm.types.llms.bedrock import (
    MessageBlock as BedrockMessageBlock,
)
from litellm.types.llms.openai import (
    ChatCompletionAssistantMessage,
    ChatCompletionUserMessage,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _deduplicate_content(input_value: dict[str, Json]) -> Json:
    blocks = cast("list[BedrockContentBlock]", input_value["blocks"])
    block_key = cast(str, input_value["block_key"])
    id_key = cast(str, input_value.get("id_key", "toolUseId"))
    return cast(Json, _deduplicate_bedrock_content_blocks(blocks, block_key, id_key))


def _deduplicate_tool_content(input_value: dict[str, Json]) -> Json:
    tool_content = cast("list[BedrockContentBlock]", input_value["tool_content"])
    return cast(Json, _deduplicate_bedrock_tool_content(tool_content))


def _rename_documents(input_value: dict[str, Json]) -> Json:
    contents = cast("list[BedrockMessageBlock]", input_value["contents"])
    return cast(Json, _rename_duplicate_bedrock_document_names(contents))


def _with_text(input_value: dict[str, Json]) -> Json:
    message = cast(BedrockMessageBlock, input_value["message"])
    return cast(Json, _with_text_when_document_only(message))


def _ensure_text(input_value: dict[str, Json]) -> Json:
    contents = cast("list[BedrockMessageBlock]", input_value["contents"])
    return cast(Json, _ensure_document_messages_have_text(contents))


def _sort_blocks(input_value: dict[str, Json]) -> Json:
    blocks = cast("list[BedrockContentBlock]", input_value["blocks"])
    return cast(Json, _sort_bedrock_assistant_content_blocks(blocks))


def _insert_continue(input_value: dict[str, Json]) -> Json:
    litellm.modify_params = cast(bool, input_value.get("modify_params", False))
    messages = cast("list[BedrockMessageBlock]", input_value["messages"])
    continue_message = cast(
        "str | ChatCompletionAssistantMessage | None",
        input_value.get("assistant_continue_message"),
    )
    return cast(Json, _insert_assistant_continue_message(messages, continue_message))


def _user_message(input_value: dict[str, Json]) -> Json:
    litellm.modify_params = cast(bool, input_value.get("modify_params", False))
    message = cast(ChatCompletionUserMessage, input_value["message"])
    continue_message = cast(
        "ChatCompletionUserMessage | None", input_value.get("user_continue_message")
    )
    return cast(Json, get_user_message_block_or_continue_message(message, continue_message))


def _return_continue(input_value: dict[str, Json]) -> Json:
    continue_message = cast(
        "str | ChatCompletionAssistantMessage | None",
        input_value.get("assistant_continue_message"),
    )
    return cast(Json, return_assistant_continue_message(continue_message))


def _skip_blocks(input_value: dict[str, Json]) -> Json:
    message = cast(ChatCompletionAssistantMessage, input_value["message"])
    return cast(Json, skip_empty_text_blocks(message))


def _process_blocks(input_value: dict[str, Json]) -> Json:
    message = cast(ChatCompletionAssistantMessage, input_value["message"])
    continue_message = cast(
        "str | ChatCompletionAssistantMessage | None",
        input_value.get("assistant_continue_message"),
    )
    return cast(Json, process_empty_text_blocks(message, continue_message))


def _assistant_message(input_value: dict[str, Json]) -> Json:
    litellm.modify_params = cast(bool, input_value.get("modify_params", False))
    message = cast(ChatCompletionAssistantMessage, input_value["message"])
    continue_message = cast(
        "str | ChatCompletionAssistantMessage | None",
        input_value.get("assistant_continue_message"),
    )
    return cast(
        Json,
        get_assistant_message_block_or_continue_message(message, continue_message),
    )


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "deduplicate_content": _deduplicate_content,
    "deduplicate_tool_content": _deduplicate_tool_content,
    "rename_duplicate_documents": _rename_documents,
    "with_text_when_document_only": _with_text,
    "ensure_document_messages_have_text": _ensure_text,
    "sort_assistant_content": _sort_blocks,
    "insert_assistant_continue": _insert_continue,
    "user_message_block_or_continue": _user_message,
    "return_assistant_continue": _return_continue,
    "skip_empty_text_blocks": _skip_blocks,
    "process_empty_text_blocks": _process_blocks,
    "assistant_message_or_continue": _assistant_message,
}


def run(input_value: dict[str, Json]) -> Json:
    function = cast(str, input_value["function"])
    handler = HANDLERS.get(function)
    if handler is None:
        raise ValueError(f"unknown bedrock content function: {function}")
    return handler(input_value)
