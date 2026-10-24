from typing import TypeAlias

from litellm.litellm_core_utils.url_utils import (
    encode_url_path_segment,
    encode_url_path_segments,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> str:
    expected = {"function", "value", "field_name"}
    if set(input_value) != expected:
        raise ValueError("url_path input fields must be function, value, and field_name")
    function = input_value["function"]
    value = input_value["value"]
    field_name = input_value["field_name"]
    if not isinstance(function, str):
        raise ValueError("function must be a string")
    if not isinstance(value, str):
        raise ValueError("value must be a string")
    if not isinstance(field_name, str):
        raise ValueError("field_name must be a string")
    if function == "segment":
        return encode_url_path_segment(value, field_name=field_name)
    if function == "segments":
        return encode_url_path_segments(value, field_name=field_name)
    raise ValueError(f"unknown url_path function: {function}")
