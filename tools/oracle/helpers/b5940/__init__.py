import os
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.llms.oci import common_utils
from pydantic import JsonValue

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

ENV_NAMES: dict[str, str] = {
    "oci_region": "OCI_REGION",
    "oci_user": "OCI_USER",
    "oci_fingerprint": "OCI_FINGERPRINT",
    "oci_tenancy": "OCI_TENANCY",
    "oci_key": "OCI_KEY",
    "oci_key_file": "OCI_KEY_FILE",
    "oci_compartment_id": "OCI_COMPARTMENT_ID",
}

FIXED_DATE = "Wed, 06 Nov 2024 08:49:37 GMT"


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def array_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def env_values(values: dict[str, Json]) -> dict[str, str]:
    selected: dict[str, str] = {}
    for field, name in ENV_NAMES.items():
        setting = values.get(field)
        if isinstance(setting, str):
            selected[name] = setting
    return selected


def base_urls(explicit: dict[str, Json], input_value: dict[str, Json]) -> Json:
    urls: list[Json] = []
    for candidate in array_value(input_value.get("api_bases"), "api_bases"):
        api_base = None if candidate is None else string_value(candidate, "api_base")
        urls.append(common_utils.get_oci_base_url(explicit, api_base))  # pyright: ignore[reportUnknownMemberType]
    return urls


def schema_output(input_value: dict[str, Json]) -> Json:
    schema = cast(JsonValue, object_value(input_value.get("schema"), "schema"))
    return {
        "refs": cast(Json, common_utils.resolve_oci_schema_refs(schema)),
        "anyof": cast(Json, common_utils.resolve_oci_schema_anyof(schema)),
        "sanitized": cast(Json, common_utils.sanitize_oci_schema(schema)),
        "python_types": cast(Json, common_utils.OCI_JSON_TO_PYTHON_TYPES),
    }


def description_output(input_value: dict[str, Json]) -> Json:
    description = string_value(input_value.get("description"), "description")
    param_schema = cast(JsonValue, object_value(input_value.get("schema"), "schema"))
    return common_utils.enrich_cohere_param_description(
        description, cast(dict[str, JsonValue], param_schema)
    )


def signature(explicit: dict[str, Json], input_value: dict[str, Json]) -> Json:
    api_base = string_value(input_value.get("api_base"), "api_base")
    request = object_value(input_value.get("request"), "request")
    headers = {
        name: string_value(value, name)
        for name, value in object_value(input_value.get("headers"), "headers").items()
    }
    with patch.object(common_utils, "formatdate", return_value=FIXED_DATE):
        signed, body = cast(
            tuple[dict[str, Json], bytes],
            common_utils.sign_with_manual_credentials(  # pyright: ignore[reportUnknownMemberType]
                headers, explicit, request, api_base
            ),
        )
    return {"headers": signed, "body": body.decode("utf-8")}


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    if function == "schema":
        return schema_output(input_value)
    if function == "description":
        return description_output(input_value)
    explicit = object_value(input_value.get("explicit"), "explicit")
    env = object_value(input_value.get("env"), "env")
    with patch.dict(os.environ, env_values(env), clear=False):
        if function == "credentials":
            return cast(Json, common_utils.resolve_oci_credentials(explicit))  # pyright: ignore[reportUnknownMemberType]
        if function == "base_url":
            return base_urls(explicit, input_value)
        if function == "sign":
            return signature(explicit, input_value)
    raise ValueError(f"unknown function {function}")
