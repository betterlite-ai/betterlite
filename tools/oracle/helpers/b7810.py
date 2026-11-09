import os
from collections.abc import Callable
from typing import TypeAlias, cast
from unittest import mock

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.milvus.vector_stores.transformation import MilvusVectorStoreConfig
from litellm.llms.pg_vector.vector_stores.transformation import PGVectorStoreConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.vector_stores import (
    VectorStoreCreateOptionalRequestParams,
    VectorStoreSearchOptionalRequestParams,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

MILVUS = MilvusVectorStoreConfig()
PG = PGVectorStoreConfig()


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


def numbers(value: Json, name: str) -> list[float]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    parsed: list[float] = []
    for item in value:
        if isinstance(item, bool) or not isinstance(item, (int, float)):
            raise TypeError(f"{name} items must be numbers")
        parsed.append(float(item))
    return parsed


class Recording:
    def __init__(self, details: dict[str, object]) -> None:
        self.model_call_details = details


def recording(details: dict[str, object]) -> Logging:
    return cast("Logging", Recording(details))


def milvus_params(call: Object) -> GenericLiteLLMParams | None:
    raw = call.get("litellm_params")
    if raw is None:
        return None
    fields = obj(raw, "litellm_params")
    return GenericLiteLLMParams(api_key=optional_text(fields, "api_key"))


def milvus_auth(call: Object) -> Json:
    api_key = optional_text(call, "api_key")
    store = MILVUS.get_auth_credentials({"api_key": api_key})  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
    return cast(Json, store.get("headers"))  # pyright: ignore[reportUnknownMemberType]


def milvus_validate(call: Object) -> Json:
    headers = strings(call, "headers")
    with mock.patch.dict(os.environ, strings(call, "env"), clear=True):
        return cast(
            Json,
            MILVUS.validate_environment(headers, milvus_params(call)),  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        )


def milvus_url(call: Object) -> Json:
    api_base = optional_text(call, "api_base")
    with mock.patch.dict(os.environ, strings(call, "env"), clear=True):
        return cast(
            Json,
            MILVUS.get_complete_url(api_base, {}),  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        )


def milvus_map(call: Object) -> Json:
    non_default = obj(call.get("non_default_params", {}), "non_default_params")
    optional = obj(call.get("optional_params", {}), "optional_params")
    return cast(
        Json,
        MILVUS.map_openai_params(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            non_default, optional, drop_params=False
        ),
    )


def milvus_search(call: Object) -> Json:
    logging = recording({})
    litellm_params = obj(call.get("litellm_params", {}), "litellm_params")
    url, body = MILVUS._search_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
        text(call, "vector_store_id"),
        text(call, "query_text"),
        numbers(call.get("query_vector"), "query_vector"),
        cast(
            VectorStoreSearchOptionalRequestParams,
            obj(call.get("optional_params", {}), "optional_params"),
        ),
        text(call, "api_base"),
        logging,
        litellm_params,
    )
    details = cast("dict[str, object]", logging.model_call_details)
    return {
        "url": url,
        "body": cast(Json, body),
        "query_text": cast(Json, details.get("input")),
        "embedding_model": cast(Json, details.get("embedding_model")),
    }


def milvus_decode(call: Object) -> Json:
    status = integer(call.get("status_code"), "status_code")
    body = text(call, "body")
    response = httpx.Response(
        status, headers=strings(call, "headers"), content=body.encode()
    )
    details: dict[str, object] = {
        "input": text(call, "query_text"),
        "optional_params": {"milvus_text_field": text(call, "text_field")},
    }
    try:
        decoded = MILVUS.transform_search_vector_store_response(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            response, recording(details)
        )
    except BaseLLMException as error:
        sent = dict(httpx.Headers(error.headers).items())  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
        return {"status_code": error.status_code, "headers": cast(Json, sent)}
    return cast(Json, decoded)


def milvus_create_request(call: Object) -> Json:
    optional = obj(call.get("optional_params", {}), "optional_params")
    url, body = MILVUS.transform_create_vector_store_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        cast(VectorStoreCreateOptionalRequestParams, optional),
        text(call, "api_base"),
    )
    return {"url": url, "body": cast(Json, body)}


def milvus_create_response(call: Object) -> Json:
    status = integer(call.get("status_code"), "status_code")
    response = httpx.Response(status, headers=strings(call, "headers"))
    return cast(
        Json,
        MILVUS.transform_create_vector_store_response(response),  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
    )


def pg_validate(call: Object) -> Json:
    params = GenericLiteLLMParams(api_key=optional_text(call, "api_key"))
    with mock.patch.dict(os.environ, strings(call, "env"), clear=True):
        return cast(
            Json,
            PG.validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
                strings(call, "headers"), params
            ),
        )


def pg_url(call: Object) -> Json:
    api_base = optional_text(call, "api_base")
    with mock.patch.dict(os.environ, strings(call, "env"), clear=True):
        return cast(
            Json,
            PG.get_complete_url(api_base, {}),  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        )


def pg_search(call: Object) -> Json:
    url, body = PG.transform_search_vector_store_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        text(call, "vector_store_id"),
        cast("str | list[str]", call["query"]),
        cast(
            VectorStoreSearchOptionalRequestParams,
            obj(call.get("params", {}), "params"),
        ),
        text(call, "api_base"),
        cast(Logging, None),
        {},
    )
    return {"url": url, "body": cast(Json, body)}


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "milvus_auth_credentials": milvus_auth,
    "milvus_validate_environment": milvus_validate,
    "milvus_complete_url": milvus_url,
    "milvus_map_params": milvus_map,
    "milvus_search_request": milvus_search,
    "milvus_decode_search": milvus_decode,
    "milvus_create_request": milvus_create_request,
    "milvus_create_response": milvus_create_response,
    "pg_vector_validate_environment": pg_validate,
    "pg_vector_complete_url": pg_url,
    "pg_vector_search_request": pg_search,
}


def run_call(value: Json) -> Json:
    call = obj(value, "call")
    kind = text(call, "call")
    handler = HANDLERS.get(kind)
    if handler is None:
        raise ValueError(f"unknown call {kind}")
    payload = obj(call.get("input"), "input")
    try:
        return {"ok": True, "value": handler(payload)}
    except Exception:  # noqa: BLE001 - one call's failure is the outcome the case compares
        return {"ok": False}


def run(input_value: dict[str, Json]) -> Json:
    calls = input_value.get("calls")
    if not isinstance(calls, list):
        raise ValueError("calls must be a list")
    return [run_call(value) for value in calls]
