import os
from collections.abc import Callable
from typing import TypeAlias, cast
from unittest import mock

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.ragflow.vector_stores.transformation import RAGFlowVectorStoreConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.vector_stores import (
    VectorStoreCreateOptionalRequestParams,
    VectorStoreSearchOptionalRequestParams,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

RAGFLOW = RAGFlowVectorStoreConfig()


def obj(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def strings(fields: Object, name: str) -> dict[str, str]:
    raw = obj(fields.get(name, {}), name)
    variables = {key: value for key, value in raw.items() if isinstance(value, str)}
    if len(variables) != len(raw):
        raise TypeError(f"{name} values must be strings")
    return variables


def integer(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError(f"{name} must be an integer")
    return value


def optional_params(fields: Object, name: str) -> Object | None:
    raw = fields.get(name)
    if raw is None:
        return None
    return obj(raw, name)


def floating_params(fields: Object) -> GenericLiteLLMParams | None:
    raw = optional_params(fields, "litellm_params")
    if raw is None:
        return None
    return GenericLiteLLMParams(api_key=optional_text(raw, "api_key"))


def litellm_params(fields: Object) -> dict[str, object]:
    raw = optional_params(fields, "litellm_params")
    if raw is None:
        return {}
    return {"api_base": optional_text(raw, "api_base")}


def auth_credentials(call: Object) -> Json:
    api_key = optional_text(call, "api_key")
    with mock.patch.dict(os.environ, strings(call, "env"), clear=True):
        credentials = RAGFLOW.get_auth_credentials({"api_key": api_key})  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
    return cast(Json, credentials.get("headers"))  # pyright: ignore[reportUnknownMemberType]


def validate_environment(call: Object) -> Json:
    headers = strings(call, "headers")
    with mock.patch.dict(os.environ, strings(call, "env"), clear=True):
        return cast(
            Json,
            RAGFLOW.validate_environment(headers, floating_params(call)),  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        )


def complete_url(call: Object) -> Json:
    api_base = optional_text(call, "api_base")
    with mock.patch.dict(os.environ, strings(call, "env"), clear=True):
        return cast(
            Json,
            RAGFLOW.get_complete_url(api_base, litellm_params(call)),  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        )


def endpoints(_call: Object) -> Json:
    return cast(Json, RAGFLOW.get_vector_store_endpoints_by_type())  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]


def create_request(call: Object) -> Json:
    optional = obj(call.get("optional_params", {}), "optional_params")
    url, body = RAGFLOW.transform_create_vector_store_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        cast(VectorStoreCreateOptionalRequestParams, optional),
        text(call, "api_base"),
    )
    return {"url": url, "body": cast(Json, body)}


def create_response(call: Object) -> Json:
    status = integer(call.get("status_code"), "status_code")
    response = httpx.Response(
        status,
        headers=strings(call, "headers"),
        content=text(call, "body").encode(),
    )
    try:
        return cast(
            Json,
            RAGFLOW.transform_create_vector_store_response(response),  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        )
    except BaseLLMException as error:
        sent = dict(httpx.Headers(error.headers).items())  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
        return {"status_code": error.status_code, "headers": cast(Json, sent)}


def search_request(_call: Object) -> Json:
    RAGFLOW.transform_search_vector_store_request(  # pyright: ignore[reportUnknownMemberType]
        "vs_a",
        "query",
        cast(VectorStoreSearchOptionalRequestParams, {}),
        "http://localhost:9380/api/v1/datasets",
        cast(Logging, None),
        {},
    )
    raise AssertionError("RAGFlow search request did not fail")


def search_response(_call: Object) -> Json:
    RAGFLOW.transform_search_vector_store_response(  # pyright: ignore[reportUnknownMemberType]
        httpx.Response(200), cast(Logging, None)
    )
    raise AssertionError("RAGFlow search response did not fail")


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "auth_credentials": auth_credentials,
    "validate_environment": validate_environment,
    "complete_url": complete_url,
    "endpoints": endpoints,
    "create_request": create_request,
    "create_response": create_response,
    "search_request": search_request,
    "search_response": search_response,
}


def run_call(value: Json) -> Json:
    call = obj(value, "call")
    kind = text(call, "call")
    handler = HANDLERS.get(kind)
    if handler is None:
        raise ValueError(f"unknown call {kind}")
    payload = obj(call.get("input", {}), "input")
    try:
        return {"ok": True, "value": handler(payload)}
    except Exception:  # noqa: BLE001 - one call's failure is the outcome the case compares
        return {"ok": False}


def run(input_value: dict[str, Json]) -> Json:
    calls = input_value.get("calls")
    if not isinstance(calls, list):
        raise ValueError("calls must be a list")
    return [run_call(value) for value in calls]
