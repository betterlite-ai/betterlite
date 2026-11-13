import os
from collections.abc import Callable, Mapping
from typing import TypeAlias, cast
from unittest import mock

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.mongodb.vector_stores.transformation import MongoDBVectorStoreConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import EmbeddingResponse
from litellm.types.vector_stores import VectorStoreSearchOptionalRequestParams

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

MONGODB = MongoDBVectorStoreConfig()


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


def integer(fields: Object, name: str) -> int:
    value = fields.get(name)
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError(f"{name} must be an integer")
    return value


def optional_number(fields: Object, name: str) -> float | None:
    value = fields.get(name)
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError(f"{name} must be a number")
    return float(value)


def optional_map(value: Json) -> dict[str, Json] | None:
    if value is None:
        return None
    return obj(value, "extra_body")


def timeout_of(litellm_params: Object) -> object:
    raw = litellm_params.get("timeout")
    if isinstance(raw, dict):
        return httpx.Timeout(None, read=optional_number(raw, "read"))
    return raw


class Recording:
    def __init__(self, details: dict[str, object]) -> None:
        self.model_call_details = details


def recording(details: dict[str, object]) -> Logging:
    return cast("Logging", Recording(details))


class StubExecutor:
    def __init__(self, vectors: list[Json]) -> None:
        self.vectors = vectors

    def embed(
        self,
        model: str,  # noqa: ARG002 - the protocol fixes these parameter names
        query: str,  # noqa: ARG002
        configuration: Mapping[str, object],  # noqa: ARG002
    ) -> EmbeddingResponse:
        data = [
            {"object": "embedding", "embedding": vector, "index": index}
            for index, vector in enumerate(self.vectors)
        ]
        return EmbeddingResponse(model="helper", data=cast("list[dict[str, object]]", data))

    async def aembed(
        self, model: str, query: str, configuration: Mapping[str, object]
    ) -> EmbeddingResponse:
        return self.embed(model, query, configuration)


def litellm_params_of(call: Object) -> dict[str, Json]:
    params = dict(obj(call.get("litellm_params", {}), "litellm_params"))
    params["timeout"] = cast(Json, timeout_of(params))
    return params


def search_options(call: Object) -> VectorStoreSearchOptionalRequestParams:
    return cast(
        VectorStoreSearchOptionalRequestParams,
        obj(call.get("optional_params", {}), "optional_params"),
    )


def mongodb_validate(call: Object) -> Json:
    raw = call.get("litellm_params")
    params = (
        None
        if raw is None
        else GenericLiteLLMParams.model_validate(obj(raw, "litellm_params"))
    )
    with mock.patch.dict(os.environ, strings(call, "env"), clear=True):
        return cast(
            Json,
            MONGODB.validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
                strings(call, "headers"), params
            ),
        )


def mongodb_url(call: Object) -> Json:
    return cast(
        Json,
        MONGODB.get_complete_url(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            optional_text(call, "api_base"), {}
        ),
    )


def mongodb_prepare(call: Object) -> Json:
    litellm_params = litellm_params_of(call)
    optional = search_options(call)
    params = MONGODB._params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
        litellm_params, optional, optional_map(call.get("extra_body"))
    )
    limit = MONGODB._limit(optional)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
    return {
        "query_text": MONGODB._query_text(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            cast("str | list[str]", call["query"])
        ),
        "embedding_model": params.require_embedding_model(),  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        "embedding_config": cast(Json, dict(params.litellm_embedding_config or {})),  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
        "database": params.require_database(),  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        "collection": params.require_collection(),  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        "text_field": cast(Json, params.text_field),  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        "embedding_field": cast(Json, params.embedding_field),  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        "num_candidates": MONGODB._num_candidates(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            limit, params.mongodb_num_candidates
        ),
        "max_num_results": limit,
        "timeout_ms": MONGODB._timeout_ms(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            timeout_of(litellm_params)
        ),
    }


def mongodb_search(call: Object) -> Json:
    embedding = cast("list[Json]", call.get("embedding", []))
    url, body = MONGODB.transform_search_vector_store_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        text(call, "vector_store_id"),
        cast("str | list[str]", call["query"]),
        search_options(call),
        text(call, "api_base"),
        recording({}),
        litellm_params_of(call),
        extra_body=optional_map(call.get("extra_body")),
        embedding_executor=StubExecutor(embedding),
    )
    return {"url": cast(Json, url), "body": cast(Json, body)}


def mongodb_decode(call: Object) -> Json:
    response = httpx.Response(200, content=text(call, "body").encode())
    return cast(
        Json,
        MONGODB.transform_search_vector_store_response(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            response, recording({})
        ),
    )


def mongodb_status(call: Object) -> Json:
    message = text(call, "message")
    status_code = integer(call, "status_code")
    try:
        MONGODB.get_error_class(message, status_code, {})  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
    except Exception as error:  # noqa: BLE001 - the raised exception is the outcome the case compares
        carried: object = getattr(error, "status_code", None)
        status = carried if isinstance(carried, int) and not isinstance(carried, bool) else None
        return {"status": status}
    return {"status": None}


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "mongodb_validate_environment": mongodb_validate,
    "mongodb_complete_url": mongodb_url,
    "mongodb_prepare_search": mongodb_prepare,
    "mongodb_search_request": mongodb_search,
    "mongodb_decode_search": mongodb_decode,
    "mongodb_status_error": mongodb_status,
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
