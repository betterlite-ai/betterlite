import os
from collections.abc import Callable, Mapping
from typing import TypeAlias, cast
from unittest import mock

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.openai.vector_store_files.transformation import OpenAIVectorStoreFilesConfig
from litellm.llms.openai.vector_stores.transformation import OpenAIVectorStoreConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.vector_store_files import (
    VectorStoreFileCreateRequest,
    VectorStoreFileListQueryParams,
    VectorStoreFileUpdateRequest,
)
from litellm.types.vector_stores import (
    VectorStoreCreateOptionalRequestParams,
    VectorStoreSearchOptionalRequestParams,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

STORE = OpenAIVectorStoreConfig()
FILES = OpenAIVectorStoreFilesConfig()
STORE_ID = "vs"


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected object")
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
    raw = obj(fields.get(name, {}))
    variables = {key: value for key, value in raw.items() if isinstance(value, str)}
    if len(variables) != len(raw):
        raise TypeError(f"{name} values must be strings")
    return variables


def request(url: str, body: Mapping[str, object]) -> Json:
    return {"url": url, "body": cast(Json, dict(body))}


def auth(fields: Object) -> Json:
    params: dict[str, object] = {"api_key": optional_text(fields, "api_key")}
    store = STORE.get_auth_credentials(params)  # pyright: ignore[reportUnknownMemberType]
    if store != FILES.get_auth_credentials(params):
        raise AssertionError("store and file auth credentials diverged")
    return cast(Json, store.get("headers"))  # pyright: ignore[reportUnknownMemberType]


def validate(fields: Object) -> Json:
    params = GenericLiteLLMParams(api_key=optional_text(fields, "api_key"))
    with mock.patch.dict(os.environ, strings(fields, "env"), clear=True):
        litellm.api_key = optional_text(fields, "litellm_api_key")
        litellm.openai_key = optional_text(fields, "litellm_openai_key")
        store = STORE.validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            strings(fields, "headers"), params
        )
        files = FILES.validate_environment(
            headers=strings(fields, "headers"), litellm_params=params
        )
    if store != files:
        raise AssertionError("store and file validate_environment diverged")
    return cast(Json, store)


def complete_url(fields: Object) -> Json:
    api_base = optional_text(fields, "api_base")
    with mock.patch.dict(os.environ, strings(fields, "env"), clear=True):
        litellm.api_base = optional_text(fields, "litellm_api_base")
        return {
            "store": STORE.get_complete_url(api_base, {}),  # pyright: ignore[reportUnknownMemberType]
            "files": FILES.get_complete_url(
                api_base=api_base,
                vector_store_id=text(fields, "vector_store_id"),
                litellm_params={},
            ),
        }


def pairs(endpoints: Mapping[str, object]) -> Json:
    return cast(
        Json,
        {
            kind: [list(pair) for pair in cast(list[tuple[str, str]], value)]
            for kind, value in endpoints.items()
        },
    )


def endpoints(fields: Object) -> Json:
    if text(fields, "surface") == "store":
        return pairs(cast(Mapping[str, object], STORE.get_vector_store_endpoints_by_type()))
    return pairs(FILES.get_vector_store_file_endpoints_by_type())


def search(fields: Object) -> Json:
    url, body = STORE.transform_search_vector_store_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        text(fields, "vector_store_id"),
        cast(str | list[str], fields["query"]),
        cast(VectorStoreSearchOptionalRequestParams, fields.get("params", {})),
        text(fields, "api_base"),
        cast(Logging, None),
        {},
    )
    return request(url, body)  # pyright: ignore[reportUnknownArgumentType]


def create(fields: Object) -> Json:
    params = cast(VectorStoreCreateOptionalRequestParams, fields.get("params", {}))
    url, body = STORE.transform_create_vector_store_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        params, text(fields, "api_base")
    )
    return request(url, body)  # pyright: ignore[reportUnknownArgumentType]


def file_create(fields: Object) -> Json:
    return request(
        *FILES.transform_create_vector_store_file_request(
            vector_store_id=STORE_ID,
            create_request=cast(VectorStoreFileCreateRequest, fields["params"]),
            api_base=text(fields, "api_base"),
        )
    )


def file_list(fields: Object) -> Json:
    return request(
        *FILES.transform_list_vector_store_files_request(
            vector_store_id=STORE_ID,
            query_params=cast(VectorStoreFileListQueryParams, fields["params"]),
            api_base=text(fields, "api_base"),
        )
    )


def file_item(fields: Object) -> Json:
    operations = {
        "retrieve": FILES.transform_retrieve_vector_store_file_request,
        "content": FILES.transform_retrieve_vector_store_file_content_request,
        "delete": FILES.transform_delete_vector_store_file_request,
    }
    build = operations[text(fields, "operation")]
    return request(
        *build(
            vector_store_id=STORE_ID,
            file_id=text(fields, "file_id"),
            api_base=text(fields, "api_base"),
        )
    )


def file_update(fields: Object) -> Json:
    return request(
        *FILES.transform_update_vector_store_file_request(
            vector_store_id=STORE_ID,
            file_id=text(fields, "file_id"),
            update_request=cast(VectorStoreFileUpdateRequest, fields["params"]),
            api_base=text(fields, "api_base"),
        )
    )


def reply(fields: Object) -> Json:
    status = fields["status_code"]
    if not isinstance(status, int):
        raise TypeError("status_code must be an integer")
    response = httpx.Response(
        status, headers=strings(fields, "headers"), content=text(fields, "body").encode()
    )
    logging = cast(Logging, None)
    transforms: dict[str, list[Callable[[], object]]] = {
        "search": [lambda: STORE.transform_search_vector_store_response(response, logging)],
        "create": [lambda: STORE.transform_create_vector_store_response(response)],
        "file": [
            lambda: FILES.transform_create_vector_store_file_response(response=response),
            lambda: FILES.transform_list_vector_store_files_response(response=response),
            lambda: FILES.transform_retrieve_vector_store_file_response(response=response),
            lambda: FILES.transform_retrieve_vector_store_file_content_response(response=response),
            lambda: FILES.transform_update_vector_store_file_response(response=response),
            lambda: FILES.transform_delete_vector_store_file_response(response=response),
        ],
    }
    try:
        decoded = [transform() for transform in transforms[text(fields, "kind")]]
    except BaseLLMException as error:
        sent = dict(httpx.Headers(error.headers).items())  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
        return {"status_code": error.status_code, "headers": cast(Json, sent)}
    if any(item != decoded[0] for item in decoded):
        raise AssertionError("reply transforms diverged")
    return cast(Json, decoded[0])


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "auth_credentials": auth,
    "validate_environment": validate,
    "complete_url": complete_url,
    "endpoints": endpoints,
    "search_request": search,
    "create_request": create,
    "file_create_request": file_create,
    "file_list_request": file_list,
    "file_item_request": file_item,
    "file_update_request": file_update,
    "reply": reply,
}


def run(argument: Json) -> Json:
    fields = obj(argument)
    if len(fields) != 1:
        raise ValueError("expected one openai_vector_stores variant")
    name, value = next(iter(fields.items()))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown openai_vector_stores case: {name}")
    return handler(obj(value))
