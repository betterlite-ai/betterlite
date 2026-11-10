from typing import TypeAlias, cast

from litellm.litellm_core_utils.prompt_templates.factory import (
    _bedrock_tools_pt,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
    _is_bedrock_tool_block,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
    add_cache_point_tool_block,  # pyright: ignore[reportUnknownVariableType]
    make_valid_bedrock_tool_name,
)
from litellm.llms.bedrock.common_utils import get_bedrock_tool_name

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _model_name(input_value: dict[str, Json]) -> str | None:
    model = input_value.get("model")
    return model if isinstance(model, str) else None


def _valid_names(input_value: dict[str, Json]) -> Json:
    entries: list[Json] = []
    for name in cast(list[str], input_value["names"]):
        valid = make_valid_bedrock_tool_name(name)
        entries.append(
            {"input": name, "valid": valid, "reverse": get_bedrock_tool_name(valid)}
        )
    return entries


def _is_native(input_value: dict[str, Json]) -> Json:
    tools = cast(list[Json], input_value["tools"])
    return [
        _is_bedrock_tool_block(cast("dict[str, Json]", tool)) for tool in tools
    ]


def _cache_point(input_value: dict[str, Json]) -> Json:
    results: list[Json] = []
    for scenario in cast(list[dict[str, Json]], input_value["scenarios"]):
        tool = cast("dict[str, Json]", scenario["tool"])
        results.append(
            cast(Json, add_cache_point_tool_block(tool, model=_model_name(scenario)))
        )
    return results


def _tools(input_value: dict[str, Json]) -> Json:
    tools = cast(list[Json], input_value["tools"])
    blocks = cast(
        "list[dict[str, Json]]", _bedrock_tools_pt(tools, model=_model_name(input_value))
    )
    reverse: dict[str, str] = {}
    for block in blocks:
        spec = block.get("toolSpec")
        if isinstance(spec, dict):
            name = spec.get("name")
            if isinstance(name, str):
                reverse[name] = get_bedrock_tool_name(name)
    return cast(Json, {"tools": blocks, "reverse": reverse})


def run(input_value: dict[str, Json]) -> Json:
    match input_value.get("call"):
        case "valid_names":
            return _valid_names(input_value)
        case "is_native":
            return _is_native(input_value)
        case "cache_point":
            return _cache_point(input_value)
        case "tools":
            return _tools(input_value)
        case other:
            raise ValueError(f"unknown bedrock_tools call: {other!r}")
