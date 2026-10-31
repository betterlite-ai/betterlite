from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.litellm_core_utils.prompt_templates.common_utils import (
    drop_non_python_regex_patterns,
    flatten_combinators_and_drop_non_python_regex_patterns,
    flatten_top_level_schema_combinators,
    tool_with_sanitized_parameters,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _flatten_top_level(input_value: dict[str, Json]) -> Json:
    schema = cast(dict[str, Json], input_value["schema"])
    return cast(Json, flatten_top_level_schema_combinators(schema))


def _drop_non_python_regex(input_value: dict[str, Json]) -> Json:
    schema = cast(dict[str, Json], input_value["schema"])
    return cast(Json, drop_non_python_regex_patterns(schema))


def _flatten_and_drop(input_value: dict[str, Json]) -> Json:
    schema = cast(dict[str, Json], input_value["schema"])
    return cast(Json, flatten_combinators_and_drop_non_python_regex_patterns(schema))


def _sanitize_tool(input_value: dict[str, Json]) -> Json:
    tool = cast(dict[str, Json], input_value["tool"])
    return cast(
        Json,
        tool_with_sanitized_parameters(
            tool, flatten_combinators_and_drop_non_python_regex_patterns
        ),
    )


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "flatten_top_level": _flatten_top_level,
    "drop_non_python_regex_patterns": _drop_non_python_regex,
    "flatten_combinators_and_drop_non_python_regex_patterns": _flatten_and_drop,
    "tool_with_sanitized_parameters": _sanitize_tool,
}


def run(input_value: dict[str, Json]) -> Json:
    mode = cast(str, input_value["mode"])
    if mode not in HANDLERS:
        raise ValueError(f"unsupported mode {mode}")
    return HANDLERS[mode](input_value)
