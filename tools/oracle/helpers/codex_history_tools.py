import json
from typing import TypeAlias, cast

from litellm.llms.base_llm.responses.codex_compat import (
    drop_unsupported_tools,
    normalize_codex_input_items,
)
from litellm.responses.additional_tools import hoist_additional_tools
from litellm.types.llms.openai import ALL_RESPONSES_API_TOOL_PARAMS, ResponseInputParam

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class InputError(ValueError):
    pass


def _parse_function_call_arguments(items: Json) -> Json:
    if not isinstance(items, list):
        return items
    parsed: list[Json] = []
    for item in items:
        parsed_item = item
        if isinstance(item, dict) and item.get("type") == "function_call":
            arguments = item.get("arguments")
            if isinstance(arguments, str):
                parsed_item = {**item, "arguments": cast(Json, json.loads(arguments))}
        parsed.append(parsed_item)
    return parsed


def _normalize(value: Json) -> dict[str, Json]:
    normalized, rewritten_types = normalize_codex_input_items(cast(ResponseInputParam, value))
    return {
        "input": _parse_function_call_arguments(cast(Json, normalized)),
        "rewritten_types": list(rewritten_types),
    }


def _drop_unsupported_tools(tools: Json, supported_types: Json) -> dict[str, Json]:
    kept, dropped_types = drop_unsupported_tools(
        cast("list[Json]", tools), frozenset(cast("list[str]", supported_types))
    )
    return {"tools": cast(Json, list(kept)), "dropped_types": cast(Json, list(dropped_types))}


def _hoist(value: Json, tools: Json) -> dict[str, Json]:
    result = hoist_additional_tools(
        cast(ResponseInputParam, value),
        cast("list[ALL_RESPONSES_API_TOOL_PARAMS] | None", tools),
    )
    return {
        "input": cast(Json, result.input),
        "tools": cast(Json, list(result.tools)),
        "hoisted": cast(Json, list(result.hoisted)),
    }


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    if operation == "normalize":
        result = _normalize(input_value.get("input"))
    elif operation == "drop_unsupported_tools":
        result = _drop_unsupported_tools(
            input_value.get("tools", []), input_value.get("supported_types", [])
        )
    elif operation == "hoist_additional_tools":
        result = _hoist(input_value.get("input"), input_value.get("tools"))
    else:
        raise InputError("unknown codex_history_tools operation")
    return cast(Json, json.loads(json.dumps(result)))
