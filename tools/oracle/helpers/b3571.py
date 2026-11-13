import sys
from collections.abc import Generator
from contextlib import contextmanager
from types import ModuleType
from typing import Final, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.constants import DEFAULT_REQUEST_TIMEOUT_SECONDS
from litellm.passthrough.timeout_utils import (
    resolve_llm_passthrough_timeout,
    resolve_pass_through_request_timeout,
)
from litellm.passthrough.utils import BasePassthroughUtils

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
PROXY_MODULE: Final = "litellm.proxy.proxy_server"


def object_value(value: Json | None, field: str) -> Object:
    if not isinstance(value, dict):
        raise ValueError(f"{field} must be an object")
    return value


def string_value(value: Json | None, field: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{field} must be a string")
    return value


def number_value(value: Json | None, field: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{field} must be a number")
    return float(value)


def optional_number(value: Json | None, field: str) -> float | None:
    if value is None:
        return None
    return number_value(value, field)


def string_map(value: Json | None, field: str) -> dict[str, str]:
    fields = object_value(value, field)
    result: dict[str, str] = {}
    for key, item in fields.items():
        result[key] = string_value(item, f"{field}.{key}")
    return result


def query_item(value: Json | None, field: str) -> str | list[str]:
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return [string_value(entry, field) for entry in value]
    raise ValueError(f"{field} must be a string or a list of strings")


def query_map(value: Json | None, field: str) -> dict[str, str | list[str]]:
    fields = object_value(value, field)
    result: dict[str, str | list[str]] = {}
    for key, item in fields.items():
        result[key] = query_item(item, f"{field}.{key}")
    return result


def timeout_value(value: Json | None, field: str) -> float | str:
    if isinstance(value, bool) or not isinstance(value, (int, float, str)):
        raise ValueError(f"{field} must be a number or a string")
    if isinstance(value, str):
        return value
    return float(value)


def request_kwargs(fields: Object) -> dict[str, object]:
    kwargs: dict[str, object] = {}
    stream = fields.get("stream")
    if stream is not None:
        if not isinstance(stream, bool):
            raise ValueError("request.stream must be a boolean")
        kwargs["stream"] = stream
    for key in ("stream_timeout", "timeout", "request_timeout"):
        value = fields.get(key)
        if value is not None:
            kwargs[key] = timeout_value(value, f"request.{key}")
    return kwargs


def deployment_params(fields: Object) -> dict[str, object]:
    params: dict[str, object] = {}
    for key in ("stream_timeout", "timeout", "request_timeout"):
        value = fields.get(key)
        if value is not None:
            params[key] = timeout_value(value, f"deployment.{key}")
    return params


def router_value(fields: Object, key: str) -> float | str | None:
    value = fields.get(key)
    if value is None:
        return None
    return timeout_value(value, f"router.{key}")


def settings_mapping(settings: float | None) -> dict[str, Json]:
    if settings is None:
        return {}
    return {"pass_through_request_timeout": settings}


@contextmanager
def proxy_module(settings: float | None) -> Generator[None, None, None]:
    module = ModuleType(PROXY_MODULE)
    module.general_settings = settings_mapping(settings)  # pyright: ignore[reportAttributeAccessIssue]
    previous = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = module
    try:
        yield
    finally:
        if previous is None:
            _ = sys.modules.pop(PROXY_MODULE, None)
        else:
            sys.modules[PROXY_MODULE] = previous


def query_merge(input_value: Object) -> Json:
    existing_url = string_value(input_value.get("existing_url"), "existing_url")
    request_params = query_map(input_value.get("request_query_params", {}), "request_query_params")
    defaults = input_value.get("default_query_params")
    default_params = None if defaults is None else query_map(defaults, "default_query_params")
    merged = BasePassthroughUtils.get_merged_query_parameters(  # pyright: ignore[reportUnknownMemberType]
        existing_url=httpx.URL(existing_url),
        request_query_params=request_params,
        default_query_params=default_params,
    )
    return cast("dict[str, Json]", dict(merged))  # pyright: ignore[reportUnknownVariableType]


def forward_headers(input_value: Object) -> Json:
    request_headers = string_map(input_value.get("request_headers", {}), "request_headers")
    custom_headers = string_map(input_value.get("custom_headers", {}), "custom_headers")
    forward = input_value.get("forward_headers")
    if forward is not None and not isinstance(forward, bool):
        raise ValueError("forward_headers must be a boolean")
    merged = BasePassthroughUtils.forward_headers_from_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        request_headers=request_headers,
        headers=custom_headers,
        forward_headers=forward,
    )
    return cast("dict[str, Json]", merged)


def proxy_timeout(input_value: Object) -> Json:
    endpoint = optional_number(input_value.get("endpoint_timeout"), "endpoint_timeout")
    settings = optional_number(input_value.get("settings_timeout"), "settings_timeout")
    with proxy_module(settings):
        return resolve_pass_through_request_timeout(endpoint_timeout=endpoint)


def sdk_timeout(input_value: Object) -> Json:
    request = request_kwargs(object_value(input_value.get("request", {}), "request"))
    params = deployment_params(object_value(input_value.get("deployment", {}), "deployment"))
    router = object_value(input_value.get("router", {}), "router")
    configured = optional_number(input_value.get("configured_timeout"), "configured_timeout")
    settings = optional_number(input_value.get("settings_timeout"), "settings_timeout")
    request_timeout = DEFAULT_REQUEST_TIMEOUT_SECONDS if configured is None else configured
    with (
        proxy_module(settings),
        patch.object(litellm, "request_timeout", request_timeout),
        patch.object(litellm, "request_timeout_explicitly_set", configured is not None),
    ):
        return resolve_llm_passthrough_timeout(
            kwargs=request,
            litellm_params=params,
            router_timeout=router_value(router, "timeout"),
            router_stream_timeout=router_value(router, "stream_timeout"),
        )


def run(input_value: dict[str, Json]) -> Json:
    operation = string_value(input_value.get("operation"), "operation")
    if operation == "query_merge":
        return query_merge(input_value)
    if operation == "forward_headers":
        return forward_headers(input_value)
    if operation == "proxy_timeout":
        return proxy_timeout(input_value)
    if operation == "sdk_timeout":
        return sdk_timeout(input_value)
    raise ValueError(f"unknown passthrough policy operation {operation}")
