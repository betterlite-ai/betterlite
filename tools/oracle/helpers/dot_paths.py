from typing import TypeAlias

from litellm.litellm_core_utils.dot_notation_indexing import (
    delete_nested_value,
    get_nested_value,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def object_fields(value: Json, expected: set[str], name: str) -> dict[str, Json]:
    if not isinstance(value, dict) or set(value) != expected:
        fields = ", ".join(sorted(expected))
        raise ValueError(f"{name} must be an object with the fields {fields}")
    return value


def string_field(fields: dict[str, Json], name: str) -> str:
    value = fields[name]
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def data_field(fields: dict[str, Json]) -> dict[str, Json]:
    data = fields["data"]
    if not isinstance(data, dict):
        raise ValueError("data must be an object")
    return data


def run(input_value: dict[str, Json]) -> object:
    if len(input_value) != 1:
        raise ValueError("dot_paths input must name exactly one function")
    function, arguments = next(iter(input_value.items()))
    if function == "get":
        fields = object_fields(arguments, {"data", "key_path", "default"}, "get")
        key_path = string_field(fields, "key_path")
        return get_nested_value(data_field(fields), key_path, fields["default"])
    if function == "delete":
        fields = object_fields(arguments, {"data", "path"}, "delete")
        return delete_nested_value(data_field(fields), string_field(fields, "path"))
    raise ValueError(f"unknown dot_paths function: {function}")
