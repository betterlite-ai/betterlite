from typing import TypeAlias, cast

from litellm.llms.anthropic.chat.transformation import (
    AnthropicConfig,
    _basic_sanitize_anthropic_tool_name,  # pyright: ignore[reportPrivateUsage]
    _build_anthropic_tool_name_maps,  # pyright: ignore[reportPrivateUsage]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "basic":
        return cast(Json, _basic_sanitize_anthropic_tool_name(cast(str, input_value.get("name"))))
    if function == "map":
        names = input_value.get("names")
        if not isinstance(names, list):
            raise ValueError("names must be a list")
        forward, reverse = _build_anthropic_tool_name_maps(cast(list[str], names))
        return cast(Json, {"forward": forward, "reverse": reverse})
    if function == "request":
        params = input_value.get("params")
        if not isinstance(params, dict):
            raise ValueError("params must be an object")
        forward, reverse = AnthropicConfig._sanitize_tool_names_in_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            cast(dict[str, object], params)
        )  # pyright: ignore[reportPrivateUsage]
        return cast(Json, {"params": params, "maps": {"forward": forward, "reverse": reverse}})
    if function == "messages":
        messages = input_value.get("messages")
        forward = input_value.get("forward")
        if not isinstance(messages, list) or not isinstance(forward, dict):
            raise ValueError("messages and forward must have their expected shapes")
        rewritten = AnthropicConfig._rewrite_tool_names_in_messages(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            messages, cast(dict[str, str], forward)  # pyright: ignore[reportArgumentType]
        )
        return cast(Json, {"messages": rewritten})
    raise ValueError(f"unknown anthropic tool names function: {function}")
