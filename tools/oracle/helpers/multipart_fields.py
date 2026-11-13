from typing import TypeAlias, cast

from litellm.litellm_core_utils.llm_request_utils import (
    flatten_form_field_values,
    serialize_multipart_form_fields,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


def object_value(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def object_list(value: Json, name: str) -> list[Object]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [object_value(entry, name) for entry in value]


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def _form_value(value: str | tuple[str, ...]) -> Json:
    if isinstance(value, tuple):
        return cast(Json, list(value))
    return value


def _flatten(fields: Object) -> Json:
    sources = object_list(fields.get("sources", []), "sources")
    rendered: list[Json] = []
    for name, value in flatten_form_field_values(*sources):
        rendered.append([name, _form_value(value)])
    return {"fields": rendered}


def _serialize(fields: Object) -> Json:
    data = object_value(fields.get("data", {}), "data")
    rendered: list[Json] = []
    for name, (_file_name, serialized) in serialize_multipart_form_fields(data):
        rendered.append([name, [None, serialized]])
    return {"fields": rendered}


def run(input_value: Object) -> Json:
    operation = string_value(input_value.get("operation"), "operation")
    if operation == "flatten":
        return _flatten(input_value)
    if operation == "serialize":
        return _serialize(input_value)
    raise ValueError(f"unknown multipart fields operation: {operation}")
