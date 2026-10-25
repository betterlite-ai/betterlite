from typing import TypeAlias

from litellm.litellm_core_utils.url_utils import (
    encode_url_path_segment,
    encode_url_path_segments,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def run(input_value: dict[str, Json]) -> str:
    if set(input_value) != {"domain", "operation"}:
        raise ValueError("management input fields must be domain and operation")
    domain = string_value(input_value["domain"], "domain")
    if domain != "models":
        raise ValueError(f"unknown management domain: {domain}")
    operation = object_value(input_value["operation"], "operation")
    if set(operation) != {"path_encoding"}:
        raise ValueError("unknown models management operation")
    path_input = object_value(operation["path_encoding"], "path_encoding")
    if set(path_input) != {"function", "value", "field_name"}:
        raise ValueError(
            "path_encoding input fields must be function, value, and field_name"
        )
    function = string_value(path_input["function"], "function")
    value = string_value(path_input["value"], "value")
    field_name = string_value(path_input["field_name"], "field_name")
    if function == "segment":
        return encode_url_path_segment(value, field_name=field_name)
    if function == "segments":
        return encode_url_path_segments(value, field_name=field_name)
    raise ValueError(f"unknown path encoding function: {function}")
