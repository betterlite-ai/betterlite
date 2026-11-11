from types import MappingProxyType
from typing import TypeAlias, cast

from litellm.litellm_core_utils.get_litellm_params import (
    InvalidControlOption,
    add_trusted_model_credentials_to_litellm_params,
    parse_control_options,
)
from litellm.litellm_core_utils.llm_request_utils import (
    _ensure_extra_body_is_safe,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

_EXPLICIT_KEYS = ("model", "api_key", "api_base", "custom_llm_provider")


def object_value(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def object_list(value: Json, name: str) -> list[Object]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [object_value(entry, name) for entry in value]


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


def boolean_value(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise TypeError(f"{name} must be a boolean")
    return value


def _parse(fields: Object) -> Json:
    kwargs: dict[str, object] = {}
    if "stream_chunk_size" in fields:
        kwargs["stream_chunk_size"] = fields["stream_chunk_size"]
    parsed = parse_control_options(kwargs)
    if isinstance(parsed, InvalidControlOption):
        return {"valid": False, "param": parsed.param}
    return {"valid": True, "stream_chunk_size": parsed.stream_chunk_size}


def _trusted(fields: Object) -> Json:
    trusted = object_value(fields.get("trusted", {}), "trusted")
    credentials = string_map(fields.get("credentials", {}), "credentials")
    results: list[Json] = []
    for scenario_value in object_list(fields.get("scenarios", []), "scenarios"):
        trusted_flag = boolean_value(scenario_value.get("trusted"), "scenarios[].trusted")
        params: dict[str, object] = {}
        for key in _EXPLICIT_KEYS:
            if key in trusted:
                params[key] = trusted[key]
        carried: object = MappingProxyType(credentials) if trusted_flag else credentials
        add_trusted_model_credentials_to_litellm_params(
            params, {"_litellm_internal_model_credentials": carried}
        )
        credential_keys = (
            sorted(credentials) if "_litellm_internal_model_credentials" in params else None
        )
        results.append(
            {
                "trusted": trusted_flag,
                "keys": cast(Json, sorted(params)),
                "credential_keys": cast(Json, credential_keys),
            }
        )
    return {"results": results}


def _extra_body(fields: Object) -> Json:
    body = fields.get("body")
    if body is None:
        return {"body": None}
    return {"body": cast(Json, _ensure_extra_body_is_safe(cast("dict[str, Json]", body)))}


def run(input_value: Object) -> Json:
    operation = string_value(input_value.get("operation"), "operation")
    if operation == "parse":
        return _parse(input_value)
    if operation == "trusted_credentials":
        return _trusted(input_value)
    if operation == "extra_body":
        return _extra_body(input_value)
    raise ValueError(f"unknown provider controls operation: {operation}")
