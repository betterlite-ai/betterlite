from collections.abc import Mapping
from typing import Protocol, TypeAlias, cast

from litellm.proxy._types import ModelAccessDeniedProxyException, ProxyException, SpecialHeaders

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class SupportsToDict(Protocol):
    def to_dict(self) -> Mapping[str, Json]: ...


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_string(value: Json, name: str) -> str | None:
    if value is None:
        return None
    return string_value(value, name)


def code_value(value: Json) -> int | str | None:
    if value is None or isinstance(value, str):
        return value
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError("code must be an integer, a string or null")
    return value


def header_map(value: Json) -> dict[str, str]:
    headers = object_value(value, "headers")
    return {name: string_value(item, name) for name, item in headers.items()}


def optional_header_map(value: Json) -> dict[str, str] | None:
    if value is None:
        return None
    return header_map(value)


def optional_provider_fields(value: Json) -> dict[str, Json] | None:
    if value is None:
        return None
    return object_value(value, "provider_specific_fields")


def run_exception(input_value: dict[str, Json]) -> Json:
    exception = ProxyException(
        message=string_value(input_value.get("message"), "message"),
        type=string_value(input_value.get("type"), "type"),
        param=optional_string(input_value.get("param"), "param"),
        code=code_value(input_value.get("code")),
        headers=optional_header_map(input_value.get("headers")),
        openai_code=optional_string(input_value.get("openai_code"), "openai_code"),
        provider_specific_fields=optional_provider_fields(
            input_value.get("provider_specific_fields")
        ),
    )
    openai_code = exception.openai_code
    return {
        "code": exception.code,
        "openai_code": None if openai_code is None else str(openai_code),
        "body": cast(Json, dict(cast(SupportsToDict, exception).to_dict())),
        "headers": cast(Json, exception.headers),
    }


def run_access_denied(input_value: dict[str, Json]) -> Json:
    denied = ModelAccessDeniedProxyException(
        message=string_value(input_value.get("message"), "message"),
        internal_message=string_value(input_value.get("internal_message"), "internal_message"),
        type=string_value(input_value.get("type"), "type"),
        param=optional_string(input_value.get("param"), "param"),
        code=code_value(input_value.get("code")),
    )
    return {
        "sanitized": denied.sanitized_internal_message(),
        "body": cast(Json, dict(cast(SupportsToDict, denied).to_dict())),
    }


def run_credential_headers() -> Json:
    names = SpecialHeaders.litellm_credential_header_names()
    return {"names": cast(Json, sorted(names))}


def run(input_value: dict[str, Json]) -> Json:
    kind = string_value(input_value.get("kind"), "kind")
    match kind:
        case "exception":
            return run_exception(input_value)
        case "access_denied":
            return run_access_denied(input_value)
        case "credential_headers":
            return run_credential_headers()
        case _:
            raise ValueError(f"unknown kind: {kind}")
