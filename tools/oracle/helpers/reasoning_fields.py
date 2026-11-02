from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.litellm_core_utils.prompt_templates import common_utils
from litellm.types.llms.openai import (
    AllMessageValues,
    ChatCompletionRedactedThinkingBlock,
    ChatCompletionThinkingBlock,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _reasoning_content_from_thinking_blocks(input_value: dict[str, Json]) -> Json:
    thinking_blocks = cast(
        "list[ChatCompletionThinkingBlock | ChatCompletionRedactedThinkingBlock]",
        input_value["thinking_blocks"],
    )
    return common_utils.reasoning_content_from_thinking_blocks(thinking_blocks)


def _encrypted_reasoning_signature(input_value: dict[str, Json]) -> Json:
    encrypted_content = cast(str, input_value["encrypted_content"])
    return common_utils.encrypted_reasoning_signature(encrypted_content)


def _encrypted_content_from_signature(input_value: dict[str, Json]) -> Json:
    return common_utils.encrypted_content_from_signature(input_value["signature"])


def _is_encrypted_reasoning_block(input_value: dict[str, Json]) -> Json:
    return common_utils.is_encrypted_reasoning_block(input_value["block"])


def _is_unsignable_thinking_block(input_value: dict[str, Json]) -> Json:
    return common_utils.is_unsignable_thinking_block(input_value["block"])


def _strip_encrypted_reasoning_from_messages(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value["messages"])
    common_utils.strip_encrypted_reasoning_from_messages(messages)
    return cast(Json, messages)


def _responses_reasoning_items_from_thinking_blocks(input_value: dict[str, Json]) -> Json:
    thinking_blocks = cast(list[dict[str, Json]], input_value["thinking_blocks"])
    items = common_utils.responses_reasoning_items_from_thinking_blocks(thinking_blocks)
    return cast(Json, list(items))


def _extract_base64_data(input_value: dict[str, Json]) -> Json:
    image_url = cast(str, input_value["image_url"])
    return common_utils._extract_base64_data(image_url)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def _extract_reasoning_content(input_value: dict[str, Json]) -> Json:
    message = cast(dict[str, Json], input_value["message"])
    reasoning_content, content = common_utils._extract_reasoning_content(message)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    return {"reasoning_content": reasoning_content, "content": cast(Json, content)}


def _parse_content_for_reasoning(input_value: dict[str, Json]) -> Json:
    message_text = cast("str | None", input_value.get("message_text"))
    reasoning_content, content = common_utils._parse_content_for_reasoning(message_text)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {"reasoning_content": reasoning_content, "content": content}


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "reasoning_content_from_thinking_blocks": _reasoning_content_from_thinking_blocks,
    "encrypted_reasoning_signature": _encrypted_reasoning_signature,
    "encrypted_content_from_signature": _encrypted_content_from_signature,
    "is_encrypted_reasoning_block": _is_encrypted_reasoning_block,
    "is_unsignable_thinking_block": _is_unsignable_thinking_block,
    "strip_encrypted_reasoning_from_messages": _strip_encrypted_reasoning_from_messages,
    "responses_reasoning_items_from_thinking_blocks": (
        _responses_reasoning_items_from_thinking_blocks
    ),
    "extract_base64_data": _extract_base64_data,
    "extract_reasoning_content": _extract_reasoning_content,
    "parse_content_for_reasoning": _parse_content_for_reasoning,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    if operation not in HANDLERS:
        raise ValueError(f"unsupported operation {operation}")
    return HANDLERS[operation](input_value)
