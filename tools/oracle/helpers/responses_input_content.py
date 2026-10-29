from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.responses.litellm_completion_transformation.transformation import (
    LiteLLMCompletionResponsesConfig,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _resolve_file_id(input_value: dict[str, Json]) -> Json:
    item = cast(dict[str, object], input_value["item"])
    return cast(Json, LiteLLMCompletionResponsesConfig._resolve_file_id(item))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def _input_file_block(input_value: dict[str, Json]) -> Json:
    item = cast(dict[str, object], input_value["item"])
    return cast(
        Json,
        LiteLLMCompletionResponsesConfig._transform_input_file_item_to_file_item(item),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )


def _input_image_block(input_value: dict[str, Json]) -> Json:
    item = cast(dict[str, str], input_value["item"])
    return cast(
        Json,
        dict(
            LiteLLMCompletionResponsesConfig._transform_input_image_item_to_image_item(item)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        ),
    )


def _content_blocks(input_value: dict[str, Json]) -> Json:
    content = input_value["content"]
    transform = LiteLLMCompletionResponsesConfig._transform_responses_api_content_to_chat_completion_content  # noqa: SLF001, E501  # pyright: ignore[reportPrivateUsage]
    return cast(Json, transform(content))


def _content_types(input_value: dict[str, Json]) -> Json:
    content_types = cast(list[str], input_value["content_types"])
    instructions = cast(list[str | None], input_value["instructions"])
    get_content_type = LiteLLMCompletionResponsesConfig._get_chat_completion_request_content_type  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return cast(
        Json,
        {
            "content_types": [
                get_content_type(content_type) for content_type in content_types
            ],
            "instructions": [
                dict(
                    LiteLLMCompletionResponsesConfig.transform_instructions_to_system_message(
                        instruction
                    )
                )
                for instruction in instructions
            ],
        },
    )


def _instructions_to_system_message(input_value: dict[str, Json]) -> Json:
    instructions = cast(str | None, input_value.get("instructions"))
    return cast(
        Json,
        dict(
            LiteLLMCompletionResponsesConfig.transform_instructions_to_system_message(
                instructions
            )
        ),
    )


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "resolve_file_id": _resolve_file_id,
    "file_block": _input_file_block,
    "image_block": _input_image_block,
    "content_blocks": _content_blocks,
    "content_types": _content_types,
    "instructions_to_system_message": _instructions_to_system_message,
}


def run(input_value: dict[str, Json]) -> Json:
    mode = cast(str, input_value["mode"])
    if mode not in HANDLERS:
        raise ValueError(f"unsupported mode {mode}")
    return HANDLERS[mode](input_value)
