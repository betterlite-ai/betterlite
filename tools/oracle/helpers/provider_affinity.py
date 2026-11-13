from typing import TypeAlias, cast

from litellm.litellm_core_utils.provider_affinity import (
    add_provider_affinity_header,
    get_stable_session_id,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

_SESSION_KEYS = (
    "provider_affinity_header",
    "session_id",
    "litellm_session_id",
    "metadata",
    "litellm_metadata",
)


def object_value(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def string_map(value: Json, name: str) -> dict[str, str]:
    fields = object_value(value, name)
    result: dict[str, str] = {}
    for key, entry in fields.items():
        if not isinstance(entry, str):
            raise TypeError(f"{name}.{key} must be a string")
        result[key] = entry
    return result


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def _params(fields: Object) -> dict[str, object]:
    params: dict[str, object] = {}
    for key in _SESSION_KEYS:
        if key in fields:
            params[key] = fields[key]
    return params


def _add(fields: Object) -> Json:
    headers = string_map(fields.get("headers", {}), "headers")
    params = _params(object_value(fields.get("params", {}), "params"))
    return cast(Json, add_provider_affinity_header(headers, params))


def _stable(fields: Object) -> Json:
    params = _params(object_value(fields.get("params", {}), "params"))
    return {"session_id": get_stable_session_id(params)}


def run(input_value: Object) -> Json:
    operation = string_value(input_value.get("operation"), "operation")
    if operation == "add":
        return _add(input_value)
    if operation == "stable_session":
        return _stable(input_value)
    raise ValueError(f"unknown provider affinity operation: {operation}")
