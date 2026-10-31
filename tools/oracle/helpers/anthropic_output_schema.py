from typing import TypeAlias, cast

from litellm.llms.anthropic.chat.transformation import AnthropicConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    schema = input_value.get("schema")
    if function == "filter":
        typed_schema = cast(dict[str, object], schema)
        return cast(Json, AnthropicConfig.filter_anthropic_output_schema(typed_schema))
    if function == "output":
        value = (
            None
            if schema is None
            else {"type": "json_schema", "json_schema": {"schema": schema}}
        )
        result = AnthropicConfig().map_response_format_to_anthropic_output_format(  # pyright: ignore[reportUnknownMemberType]
            cast(dict[str, object] | None, value)
        )
        return cast(Json, result)
    if function == "tool":
        typed_schema = cast(dict[str, object] | None, schema)
        result = AnthropicConfig()._create_json_tool_call_for_response_format(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            typed_schema
        )
        return cast(Json, result)
    if function == "map_output":
        value = cast(dict[str, object] | None, input_value.get("value"))
        return cast(Json, AnthropicConfig().map_response_format_to_anthropic_output_format(value))  # pyright: ignore[reportUnknownMemberType]
    if function == "map_tool":
        value = cast(dict[str, object] | None, input_value.get("value"))
        result = AnthropicConfig().map_response_format_to_anthropic_tool(  # pyright: ignore[reportUnknownMemberType]
            value, {}, is_thinking_enabled=False
        )
        return cast(Json, result)
    raise ValueError(f"unknown function {function}")
