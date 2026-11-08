from typing import TypeAlias, cast

from litellm.litellm_core_utils.json_validation_rule import (
    normalize_json_schema_types,
    normalize_tool_schema,
    validate_schema,  # pyright: ignore[reportUnknownVariableType]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return cast(dict[str, Json], value)


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def run(input_value: dict[str, Json]) -> Json:
    mode = _string(input_value.get("mode"), "mode")
    if mode == "normalize":
        return cast(
            Json,
            normalize_json_schema_types(_object(input_value.get("schema"), "schema")),
        )
    if mode == "tool":
        return cast(Json, normalize_tool_schema(_object(input_value.get("tool"), "tool")))
    if mode == "validate":
        validate_schema(
            schema=_object(input_value.get("schema"), "schema"),
            response=_string(input_value.get("response"), "response"),
        )
        return True
    raise ValueError(f"unknown mode {mode}")
