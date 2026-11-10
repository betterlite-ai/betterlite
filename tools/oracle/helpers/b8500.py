import os
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.secret_managers.main import get_secret, get_secret_bool, get_secret_str

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def secret_default(value: Json) -> str | bool | None:
    if value is None or isinstance(value, (str, bool)):
        return value
    raise ValueError("default_value must be a string, a boolean or null")


def call(function: str, secret_name: str, default_value: Json) -> Json:
    default = secret_default(default_value)
    bool_default = default if isinstance(default, bool) else None
    match function:
        case "get_secret":
            return cast(Json, get_secret(secret_name, default))
        case "get_secret_str":
            return cast(Json, get_secret_str(secret_name, default))
        case "get_secret_bool":
            return cast(Json, get_secret_bool(secret_name, bool_default))
        case _:
            raise ValueError(f"unknown function: {function}")


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    secret_name = string_value(input_value.get("secret_name"), "secret_name")
    environment = {
        name: string_value(value, name)
        for name, value in object_value(input_value.get("environment", {}), "environment").items()
    }
    default_value = input_value.get("default_value")
    litellm.secret_manager_client = None
    try:
        with patch.dict(os.environ, environment, clear=True):
            value = call(function, secret_name, default_value)
    except Exception as error:  # noqa: BLE001 - the helper reports every failure as its message text
        return {"value": None, "error": str(error)}
    return {"value": value, "error": None}
