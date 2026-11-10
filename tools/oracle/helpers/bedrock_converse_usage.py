from collections.abc import Callable
from typing import TYPE_CHECKING, TypeAlias, cast

from litellm.llms.bedrock.chat.converse_transformation import AmazonConverseConfig
from pydantic import BaseModel

if TYPE_CHECKING:
    from litellm.types.llms.bedrock import (
        BedrockConverseReasoningContentBlock,
        ConverseTokenUsageBlock,
    )

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


def to_json(value: object) -> Json:
    if isinstance(value, BaseModel):
        return cast(Json, value.model_dump())
    return cast(Json, value)


def object_field(fields: Object, name: str) -> Object:
    value = fields.get(name)
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def optional_string(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_bool(fields: Object, name: str) -> bool:
    value = fields.get(name)
    if value is None:
        return False
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def optional_int(fields: Object, name: str) -> int | None:
    value = fields.get(name)
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def optional_list(fields: Object, name: str) -> list[Json] | None:
    value = fields.get(name)
    if value is None:
        return None
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def usage_block(fields: Object) -> "ConverseTokenUsageBlock":
    return cast("ConverseTokenUsageBlock", object_field(fields, "usage"))


def transform_usage(fields: Object) -> Json:
    config = AmazonConverseConfig()
    result = config.transform_usage(  # pyright: ignore[reportUnknownMemberType]
        usage_block(fields),
        reasoning_content=optional_string(fields, "reasoning_content"),
        thinking_ran=optional_bool(fields, "thinking_ran"),
        provider_reasoning_tokens=optional_int(fields, "provider_reasoning_tokens"),
    )
    tokens = AmazonConverseConfig.thinking_tokens_from_additional_fields(  # pyright: ignore[reportUnknownMemberType]
        fields.get("additional_fields")
    )
    return {"usage": to_json(result), "thinking_tokens": tokens}


def batch_usage(fields: Object) -> Json:
    config = AmazonConverseConfig()
    usage = usage_block(fields)
    result = config.usage_from_batch_output(usage)  # pyright: ignore[reportUnknownMemberType]
    return {
        "usage": to_json(result),
        "shape": config.is_converse_usage_shape(usage),  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
    }


def reasoning_blocks(fields: Object) -> Json:
    blocks = optional_list(fields, "blocks")
    if blocks is None:
        raise ValueError("blocks must be a list")
    config = AmazonConverseConfig()
    typed = cast("list[BedrockConverseReasoningContentBlock]", blocks)
    content = config._transform_reasoning_content(typed)  # noqa: SLF001  # pyright: ignore[reportUnknownMemberType, reportPrivateUsage]
    thinking = config._transform_thinking_blocks(typed)  # noqa: SLF001  # pyright: ignore[reportUnknownMemberType, reportPrivateUsage]
    return {"reasoning_content": content, "thinking_blocks": cast(Json, thinking)}


def total_tokens(fields: Object) -> Json:
    config = AmazonConverseConfig()
    block = usage_block(fields)
    live = config.transform_usage(block)  # pyright: ignore[reportUnknownMemberType]
    batch = config.usage_from_batch_output(block)  # pyright: ignore[reportUnknownMemberType]
    return [to_json(live), to_json(batch)]


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "transform_usage": transform_usage,
    "batch": batch_usage,
    "reasoning_blocks": reasoning_blocks,
    "total_tokens": total_tokens,
}


def run(input_value: dict[str, Json]) -> Json:
    [(operation, request)] = input_value.items()
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unsupported operation {operation}")
    return handler(cast(Object, request))
