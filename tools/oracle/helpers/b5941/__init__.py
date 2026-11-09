import os
from collections.abc import Callable
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.llms.oci.common_utils import OCIError
from litellm.llms.oci.embed.transformation import OCIEmbedConfig
from litellm.types.utils import EmbeddingResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Input: TypeAlias = str | list[str] | list[int] | list[list[int]]

ENV_NAMES: dict[str, str] = {
    "oci_region": "OCI_REGION",
    "oci_user": "OCI_USER",
    "oci_fingerprint": "OCI_FINGERPRINT",
    "oci_tenancy": "OCI_TENANCY",
    "oci_key": "OCI_KEY",
    "oci_key_file": "OCI_KEY_FILE",
    "oci_compartment_id": "OCI_COMPARTMENT_ID",
}


class EmbedConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, Json],
        optional_params: dict[str, Json],
        model: str,
        *,
        drop_params: bool = False,
    ) -> dict[str, Json]: ...

    def validate_environment(  # noqa: PLR0913
        self,
        headers: dict[str, Json],
        model: str,
        messages: list[Json],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        *,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> dict[str, Json]: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
    ) -> str: ...

    def transform_embedding_request(
        self,
        model: str,
        input_data: Input,
        optional_params: dict[str, Json],
        headers: dict[str, Json],
    ) -> dict[str, Json]: ...

    def transform_embedding_response(  # noqa: PLR0913, PLR0917
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: EmbeddingResponse,
        logging_obj: object,
        api_key: str | None,
        request_data: dict[str, Json],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
    ) -> EmbeddingResponse: ...


def object_value(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return cast(Object, value)


def object_or_empty(value: Json, name: str) -> Object:
    return {} if value is None else object_value(value, name)


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def int_value(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


def array_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return cast(list[Json], value)


def env_values(values: Object) -> dict[str, str]:
    selected: dict[str, str] = {}
    for field, name in ENV_NAMES.items():
        setting = values.get(field)
        if isinstance(setting, str):
            selected[name] = setting
    return selected


def supported_params(config: EmbedConfig, step: Object) -> Json:
    model = string_value(step.get("model"), "model")
    return {"model": model, "params": cast(Json, config.get_supported_openai_params(model))}


def map_params(config: EmbedConfig, step: Object) -> Json:
    non_default = object_or_empty(step.get("non_default_params"), "non_default_params")
    optional = object_or_empty(step.get("optional_params"), "optional_params")
    mapped = config.map_openai_params(dict(non_default), dict(optional), "")
    return cast(Json, mapped)


def validate_environment_step(config: EmbedConfig, step: Object) -> Json:
    explicit = object_or_empty(step.get("explicit"), "explicit")
    env = object_or_empty(step.get("env"), "env")
    headers = object_or_empty(step.get("headers"), "headers")
    with patch.dict(os.environ, env_values(env), clear=True):
        validated = config.validate_environment(
            headers=dict(headers),
            model="",
            messages=[],
            optional_params=dict(explicit),
            litellm_params={},
            api_key=None,
            api_base=None,
        )
    return cast(Json, validated)


def complete_url_step(config: EmbedConfig, step: Object) -> Json:
    explicit = object_or_empty(step.get("explicit"), "explicit")
    env = object_or_empty(step.get("env"), "env")
    api_base_value = step.get("api_base")
    api_base = None if api_base_value is None else string_value(api_base_value, "api_base")
    with (
        patch.dict(os.environ, env_values(env), clear=True),
        patch.object(litellm, "api_base", None),
    ):
        return config.get_complete_url(api_base, None, "", dict(explicit), {})


def request_step(config: EmbedConfig, step: Object) -> Json:
    explicit = object_or_empty(step.get("explicit"), "explicit")
    env = object_or_empty(step.get("env"), "env")
    model = string_value(step.get("model"), "model")
    input_data = cast(Input, step.get("input"))
    non_default = object_or_empty(step.get("non_default_params"), "non_default_params")
    optional = object_or_empty(step.get("optional_params"), "optional_params")
    mapped = config.map_openai_params(dict(non_default), {**explicit, **optional}, model)
    with patch.dict(os.environ, env_values(env), clear=True):
        body = config.transform_embedding_request(model, input_data, mapped, {})
    return cast(Json, body)


def response_step(config: EmbedConfig, step: Object) -> Json:
    status = int_value(step.get("status"), "status")
    body = string_value(step.get("body"), "body")
    raw = httpx.Response(status, content=body)
    response = config.transform_embedding_response(
        "", raw, EmbeddingResponse(), object(), None, {}, {}, {}
    )
    fields = cast(Object, response.model_dump())
    usage = object_value(fields.get("usage"), "usage")
    return {
        "model": fields.get("model"),
        "object": fields.get("object"),
        "data": fields.get("data"),
        "usage": {
            "prompt_tokens": usage.get("prompt_tokens"),
            "total_tokens": usage.get("total_tokens"),
        },
    }


METHODS: dict[str, Callable[[EmbedConfig, Object], Json]] = {
    "supported_params": supported_params,
    "map_params": map_params,
    "validate_environment": validate_environment_step,
    "complete_url": complete_url_step,
    "request": request_step,
    "response": response_step,
}


def step_output(config: EmbedConfig, raw: Json) -> Json:
    step = object_value(raw, "step")
    method = string_value(step.get("method"), "method")
    handler = METHODS.get(method)
    if handler is None:
        raise ValueError(f"unknown method {method}")
    try:
        return {"ok": True, "value": handler(config, step)}
    except OCIError as error:
        return {"ok": False, "status": error.status_code}


def run(input_value: dict[str, Json]) -> Json:
    steps = array_value(input_value.get("steps"), "steps")
    config = cast(EmbedConfig, OCIEmbedConfig())
    return [step_output(config, step) for step in steps]
