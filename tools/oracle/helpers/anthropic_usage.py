from typing import TypeAlias, cast

from litellm.llms.anthropic.chat.transformation import AnthropicConfig
from pydantic import BaseModel

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def to_json(value: object) -> Json:
    if isinstance(value, BaseModel):
        return cast(Json, value.model_dump())
    return cast(Json, value)


def run(input_value: dict[str, Json]) -> Json:
    usage = input_value.get("usage")
    if not isinstance(usage, dict):
        raise ValueError("usage must be an object")
    usage_object = cast(dict[str, object], usage)
    config = AnthropicConfig()
    result = config.calculate_usage(  # pyright: ignore[reportUnknownMemberType]
        usage_object=usage_object,
        reasoning_content=cast(str | None, input_value.get("reasoning_content")),
        completion_response=cast(dict[str, object] | None, input_value.get("completion_response")),
        speed=cast(str | None, input_value.get("speed")),
    )
    return cast(
        Json,
        {
            "usage": to_json(result),
            "is_anthropic_usage_object": config.is_anthropic_usage_object(usage_object),  # pyright: ignore[reportUnknownMemberType]
        },
    )
