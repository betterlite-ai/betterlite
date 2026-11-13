import os
from collections.abc import Callable
from typing import Final, Protocol, TypeAlias, TypedDict, cast
from unittest import mock

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.anthropic.files.transformation import AnthropicFilesConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Pairs: TypeAlias = list[list[str]]
FileValue: TypeAlias = None | bytes | tuple[str, bytes] | tuple[str, bytes, str]

FILE_FIELDS: Final[tuple[str, ...]] = (
    "id",
    "bytes",
    "created_at",
    "filename",
    "object",
    "purpose",
    "status",
    "status_details",
    "expires_at",
)
DELETED_FIELDS: Final[tuple[str, ...]] = ("id", "object", "deleted")
FILE_TUPLE: Final = 3
CLOCK: Final = "litellm.llms.anthropic.files.transformation.time.time"

CONFIG = AnthropicFilesConfig()
SUPPORTED = cast("Callable[[str], list[str]]", CONFIG.get_supported_openai_params)
COMPLETE_URL = cast("Callable[..., str]", CONFIG.get_complete_url)
VALIDATE = cast("Callable[..., Object]", CONFIG.validate_environment)
GET_ERROR_CLASS = cast("Callable[..., object]", CONFIG.get_error_class)
CREATE_REQUEST = cast("Callable[..., Object]", CONFIG.transform_create_file_request)
CREATE_RESPONSE = cast("Callable[..., object]", CONFIG.transform_create_file_response)
RETRIEVE_REQUEST = cast("Callable[..., object]", CONFIG.transform_retrieve_file_request)
RETRIEVE_RESPONSE = cast("Callable[..., object]", CONFIG.transform_retrieve_file_response)
DELETE_REQUEST = cast("Callable[..., object]", CONFIG.transform_delete_file_request)
DELETE_RESPONSE = cast("Callable[..., object]", CONFIG.transform_delete_file_response)
LIST_REQUEST = cast("Callable[..., object]", CONFIG.transform_list_files_request)
LIST_RESPONSE = cast("Callable[..., object]", CONFIG.transform_list_files_response)
CONTENT_REQUEST = cast("Callable[..., object]", CONFIG.transform_file_content_request)
CONTENT_RESPONSE = cast("Callable[..., object]", CONFIG.transform_file_content_response)


class Dumpable(Protocol):
    def model_dump(self) -> dict[str, object]: ...


class ErrorView(Protocol):
    status_code: int
    message: str
    headers: dict[str, str]


class FailureFixture(TypedDict):
    status: int
    headers: Pairs
    body: str


class ErrorFixture(TypedDict):
    status: int
    message: str
    headers: Pairs


def field(file: Dumpable, names: tuple[str, ...]) -> Object:
    values = cast("dict[str, Json]", file.model_dump())
    return {name: values[name] for name in names}


def parts_view(request: Object) -> list[Json]:
    parts: list[Json] = []
    for name, value in request.items():
        entry = cast("tuple[Json, ...]", value)
        content = entry[1]
        if isinstance(content, str):
            data: list[int] = list(content.encode())
        else:
            data = list(cast("bytes", content))
        parts.append(
            cast(
                Json,
                {
                    "name": name,
                    "filename": entry[0],
                    "content_type": entry[2] if len(entry) == FILE_TUPLE else None,
                    "content": data,
                },
            )
        )
    return parts


def pairs_view(pairs: dict[str, str]) -> Pairs:
    return [[name, value] for name, value in pairs.items()]


def pairs_object(pairs: Pairs) -> dict[str, str]:
    return dict(cast("list[tuple[str, str]]", pairs))


def raised(error: Exception) -> Object:
    return {"kind": "ValueError", "message": str(error)}


def error_view(error: ErrorView) -> Object:
    return {
        "status": error.status_code,
        "message": error.message,
        "headers": dict(error.headers),
    }


def status_view(error: ErrorView) -> Object:
    return {"kind": "status", **error_view(error)}


def response_of(
    status: int, body: bytes, pairs: Pairs, request: httpx.Request | None = None
) -> httpx.Response:
    response = httpx.Response(
        status,
        content=body,
        headers=[(name, value) for name, value in pairs],
        request=request,
    )
    if not any(name.lower() == "content-length" for name, _ in pairs):
        response.headers.pop("content-length", None)
    return response


def failure_view(fixture: FailureFixture | None, method: str) -> Json:
    if fixture is None:
        return None
    request = httpx.Request(method, "https://api.anthropic.com/v1/files")
    response = response_of(
        fixture["status"], fixture["body"].encode(), fixture["headers"], request
    )
    try:
        response.raise_for_status()
    except httpx.HTTPStatusError:
        error = cast(
            ErrorView,
            GET_ERROR_CLASS(response.text, response.status_code, response.headers),
        )
        return cast(Json, status_view(error))
    return None


def upload_file(item: Object) -> FileValue:
    if item.get("missing") is True:
        return None
    data = cast(str, item.get("data", "")).encode()
    filename = cast(str | None, item.get("filename"))
    content_type = cast(str | None, item.get("content_type"))
    if filename is not None and content_type is not None:
        return (filename, data, content_type)
    if filename is not None:
        return (filename, data)
    return data


def created(body: Json, now: int) -> Dumpable:
    response = httpx.Response(200, json=body)
    with mock.patch(CLOCK, return_value=float(now)):
        return cast(
            Dumpable,
            CREATE_RESPONSE(
                model=None,
                raw_response=response,
                logging_obj=cast(Logging, None),
                litellm_params={},
            ),
        )


def retrieved(body: Json, now: int) -> Dumpable:
    response = httpx.Response(200, json=body)
    with mock.patch(CLOCK, return_value=float(now)):
        return cast(
            Dumpable,
            RETRIEVE_RESPONSE(
                raw_response=response,
                logging_obj=cast(Logging, None),
                litellm_params={},
            ),
        )


def file_upload_multipart(argument: Json) -> Object:
    supplied = cast(Object, argument)
    now = cast(int, supplied["now"])
    uploads: list[Json] = []
    with mock.patch(CLOCK, return_value=float(now)):
        for item in cast("list[Object]", supplied["uploads"]):
            create: Object = {"file": cast(Json, upload_file(item))}
            purpose = cast(str | None, item.get("purpose"))
            if purpose is not None:
                create["purpose"] = purpose
            try:
                request = CREATE_REQUEST(
                    model="",
                    create_file_data=create,
                    optional_params={},
                    litellm_params={},
                )
            except ValueError as error:
                uploads.append({"raised": raised(error)})
                continue
            uploads.append({"parts": parts_view(request)})
    return {
        "supported": cast(Json, SUPPORTED("")),
        "uploads": uploads,
        "failure": failure_view(
            cast("FailureFixture | None", supplied.get("failure")), "POST"
        ),
    }


def native_file_object(argument: Json) -> Object:
    supplied = cast(Object, argument)
    now = cast(int, supplied["now"])
    bodies = cast("list[Json]", supplied["bodies"])
    return {
        "create": [cast(Json, field(created(body, now), FILE_FIELDS)) for body in bodies],
        "retrieve": [
            cast(Json, field(retrieved(body, now), FILE_FIELDS)) for body in bodies
        ],
    }


def created_at_timezone(argument: Json) -> Object:
    supplied = cast(Object, argument)
    now = cast(int, supplied["now"])
    stamps = [
        field(created({"created_at": value}, now), ("created_at",))["created_at"]
        for value in cast("list[Json]", supplied["values"])
    ]
    return {"created_at": stamps}


def file_list_pagination(argument: Json) -> Object:
    supplied = cast(Object, argument)
    now = cast(int, supplied["now"])
    environment = cast("dict[str, str]", supplied.get("environment", {}))
    params_api_base = cast(str | None, supplied.get("params_api_base"))
    with mock.patch.dict(os.environ, environment, clear=True):
        queries: list[Json] = []
        url = ""
        for purpose in cast("list[str | None]", supplied["purposes"]):
            url, query = cast(
                "tuple[str, dict[str, str]]",
                LIST_REQUEST(
                    purpose=purpose,
                    optional_params={},
                    litellm_params={"api_base": params_api_base},
                ),
            )
            queries.append(cast(Json, pairs_view(query)))
        response = httpx.Response(200, json=supplied["body"])
        with mock.patch(CLOCK, return_value=float(now)):
            files = cast(
                "list[Dumpable]",
                LIST_RESPONSE(
                    raw_response=response,
                    logging_obj=cast(Logging, None),
                    litellm_params={},
                ),
            )
        return {
            "url": url,
            "queries": queries,
            "files": [cast(Json, field(file, FILE_FIELDS)) for file in files],
        }


def request_view(
    transform: Callable[..., object], file_id: str, litellm_params: Object
) -> Json:
    try:
        url, query = cast(
            "tuple[str, dict[str, str]]",
            transform(
                file_id=file_id, optional_params={}, litellm_params=litellm_params
            ),
        )
    except ValueError as error:
        return {"raised": raised(error)}
    return {"url": url, "query": cast(Json, pairs_view(query))}


def file_delete_shape(argument: Json) -> Object:
    supplied = cast(Object, argument)
    environment = cast("dict[str, str]", supplied.get("environment", {}))
    params_api_base = cast(str | None, supplied.get("params_api_base"))
    failure = cast("FailureFixture | None", supplied.get("failure"))
    with mock.patch.dict(os.environ, environment, clear=True):
        params: Object = {"api_base": params_api_base}
        file_ids = cast("list[str]", supplied["file_ids"])
        retrieve: list[Json] = [
            request_view(RETRIEVE_REQUEST, file_id, params) for file_id in file_ids
        ]
        delete: list[Json] = [
            request_view(DELETE_REQUEST, file_id, params) for file_id in file_ids
        ]
        responses: list[Json] = []
        for body in cast("list[Json]", supplied["bodies"]):
            response = httpx.Response(200, json=body)
            deleted = cast(
                Dumpable,
                DELETE_RESPONSE(
                    raw_response=response,
                    logging_obj=cast(Logging, None),
                    litellm_params={},
                ),
            )
            responses.append(cast(Json, field(deleted, DELETED_FIELDS)))
    return {
        "retrieve": retrieve,
        "delete": delete,
        "responses": responses,
        "failure": failure_view(failure, "DELETE"),
    }


def file_content_bytes(argument: Json) -> Object:
    supplied = cast(Object, argument)
    environment = cast("dict[str, str]", supplied.get("environment", {}))
    params_api_base = cast(str | None, supplied.get("params_api_base"))
    with mock.patch.dict(os.environ, environment, clear=True):
        urls: list[Json] = []
        for file_id in cast("list[str]", supplied["file_ids"]):
            url, query = cast(
                "tuple[str, dict[str, str]]",
                CONTENT_REQUEST(
                    file_content_request={"file_id": file_id},
                    optional_params={},
                    litellm_params={"api_base": params_api_base},
                ),
            )
            urls.append({"url": url, "query": cast(Json, pairs_view(query))})
        response = response_of(
            cast(int, supplied["status"]),
            bytes(cast("list[int]", supplied["body"])),
            cast(Pairs, supplied["headers"]),
        )
        _ = CONTENT_RESPONSE(
            raw_response=response,
            logging_obj=cast(Logging, None),
            litellm_params={},
        )
        result: Object = {
            "status": response.status_code,
            "headers": cast(Json, dict(response.headers)),
            "body": cast(Json, list(response.content)),
        }
        error_spec = cast(ErrorFixture, supplied["error"])
        error = cast(
            ErrorView,
            GET_ERROR_CLASS(
                error_spec["message"],
                error_spec["status"],
                pairs_object(error_spec["headers"]),
            ),
        )
    return {"urls": urls, "result": result, "error": error_view(error)}


def file_beta_header(argument: Json) -> Object:
    supplied = cast(Object, argument)
    scenarios: list[Json] = []
    for scenario in cast("list[Object]", supplied["scenarios"]):
        environment = cast("dict[str, str]", scenario.get("environment", {}))
        with mock.patch.dict(os.environ, environment, clear=True):
            params: Object = {
                "api_base": cast(str | None, scenario.get("params_api_base"))
            }
            api_key = cast(str | None, scenario.get("api_key"))
            url_bases = cast("list[str | None]", scenario.get("url_bases", []))
            urls: list[Json] = [
                cast(
                    Json,
                    COMPLETE_URL(
                        api_base=base,
                        api_key=api_key,
                        model="",
                        optional_params={},
                        litellm_params=params,
                    ),
                )
                for base in url_bases
            ]
            headers: dict[str, str] = pairs_object(
                cast(Pairs, scenario.get("caller_headers", []))
            )
            entry: Object = {"urls": urls, "headers": None, "raised": None}
            try:
                entry["headers"] = VALIDATE(
                    headers=headers,
                    model="",
                    messages=[],
                    optional_params={},
                    litellm_params=params,
                    api_key=api_key,
                    api_base=cast(str | None, scenario.get("api_base")),
                )
            except ValueError as error:
                entry["raised"] = raised(error)
            error_spec = cast(ErrorFixture, scenario["error"])
            error = cast(
                ErrorView,
                GET_ERROR_CLASS(
                    error_spec["message"],
                    error_spec["status"],
                    pairs_object(error_spec["headers"]),
                ),
            )
            entry["error"] = error_view(error)
            scenarios.append(entry)
    return {"scenarios": scenarios}


HANDLERS: Final[dict[str, Callable[[Json], Object]]] = {
    "file_upload_multipart": cast("Callable[[Json], Object]", file_upload_multipart),
    "native_file_object": cast("Callable[[Json], Object]", native_file_object),
    "created_at_timezone": cast("Callable[[Json], Object]", created_at_timezone),
    "file_list_pagination": cast("Callable[[Json], Object]", file_list_pagination),
    "file_delete_shape": cast("Callable[[Json], Object]", file_delete_shape),
    "file_content_bytes": cast("Callable[[Json], Object]", file_content_bytes),
    "file_beta_header": cast("Callable[[Json], Object]", file_beta_header),
}


def run(argument: dict[str, Json]) -> dict[str, Json]:
    [(variant, payload)] = argument.items()
    handler = HANDLERS.get(variant)
    if handler is None:
        raise ValueError(f"unknown anthropic_files variant: {variant}")
    return handler(payload)
