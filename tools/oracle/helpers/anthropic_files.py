import os
from collections.abc import Callable
from typing import Final, NotRequired, Protocol, TypeAlias, TypedDict, cast
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


class BinaryView(Protocol):
    response: httpx.Response


class ErrorView(Protocol):
    status_code: int
    message: str
    headers: dict[str, str]


class UploadItem(TypedDict):
    missing: NotRequired[bool]
    filename: NotRequired[str | None]
    content_type: NotRequired[str | None]
    data: NotRequired[str]
    purpose: NotRequired[str | None]


class UploadCase(TypedDict):
    now: int
    uploads: list[UploadItem]


class FileObjectCase(TypedDict):
    now: int
    bodies: list[Json]


class CreatedAtCase(TypedDict):
    now: int
    values: list[Json]


class ListCase(TypedDict):
    environment: NotRequired[dict[str, str]]
    params_api_base: NotRequired[str | None]
    now: int
    purposes: list[str | None]
    body: Json


class DeleteCase(TypedDict):
    environment: NotRequired[dict[str, str]]
    params_api_base: NotRequired[str | None]
    file_ids: list[str]
    bodies: list[Json]


class ErrorFixture(TypedDict):
    status: int
    message: str
    headers: Pairs


class ContentCase(TypedDict):
    environment: NotRequired[dict[str, str]]
    params_api_base: NotRequired[str | None]
    file_ids: list[str]
    status: int
    headers: Pairs
    body: list[int]
    error: ErrorFixture


class BetaScenario(TypedDict):
    environment: NotRequired[dict[str, str]]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    params_api_base: NotRequired[str | None]
    url_bases: NotRequired[list[str | None]]
    caller_headers: NotRequired[Pairs]
    error: ErrorFixture


class BetaHeaderCase(TypedDict):
    scenarios: list[BetaScenario]


def dumped(file: Dumpable) -> dict[str, Json]:
    return cast("dict[str, Json]", file.model_dump())


def field(file: Dumpable, names: tuple[str, ...]) -> Object:
    values = dumped(file)
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


def upload_file(item: UploadItem) -> FileValue:
    if item.get("missing") is True:
        return None
    data = item.get("data", "").encode()
    filename = item.get("filename")
    content_type = item.get("content_type")
    if filename is not None and content_type is not None:
        return (filename, data, content_type)
    if filename is not None:
        return (filename, data)
    return data


def file_upload_multipart(argument: UploadCase) -> Object:
    uploads: list[Json] = []
    with mock.patch(CLOCK, return_value=float(argument["now"])):
        for item in argument["uploads"]:
            create: Object = {"file": cast(Json, upload_file(item))}
            purpose = item.get("purpose")
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
    return {"supported": cast(Json, SUPPORTED("")), "uploads": uploads}


def native_file_object(argument: FileObjectCase) -> Object:
    created: list[Json] = []
    retrieved: list[Json] = []
    with mock.patch(CLOCK, return_value=float(argument["now"])):
        for body in argument["bodies"]:
            response = httpx.Response(200, json=body)
            created.append(
                field(
                    cast(
                        Dumpable,
                        CREATE_RESPONSE(
                            model=None,
                            raw_response=response,
                            logging_obj=cast(Logging, None),
                            litellm_params={},
                        ),
                    ),
                    FILE_FIELDS,
                )
            )
            retrieved.append(
                field(
                    cast(
                        Dumpable,
                        RETRIEVE_RESPONSE(
                            raw_response=response,
                            logging_obj=cast(Logging, None),
                            litellm_params={},
                        ),
                    ),
                    FILE_FIELDS,
                )
            )
    return {"create": created, "retrieve": retrieved}


def created_at_timezone(argument: CreatedAtCase) -> Object:
    values: list[Json] = []
    with mock.patch(CLOCK, return_value=float(argument["now"])):
        for value in argument["values"]:
            response = httpx.Response(200, json={"created_at": value})
            parsed = cast(
                Dumpable,
                CREATE_RESPONSE(
                    model=None,
                    raw_response=response,
                    logging_obj=cast(Logging, None),
                    litellm_params={},
                ),
            )
            values.append(dumped(parsed)["created_at"])
    return {"created_at": values}


def file_list_pagination(argument: ListCase) -> Object:
    with mock.patch.dict(os.environ, argument.get("environment", {}), clear=True):
        queries: list[Json] = []
        url = ""
        for purpose in argument["purposes"]:
            url, query = cast(
                "tuple[str, dict[str, str]]",
                LIST_REQUEST(
                    purpose=purpose,
                    optional_params={},
                    litellm_params={"api_base": argument.get("params_api_base")},
                ),
            )
            queries.append(cast(Json, pairs_view(query)))
        with mock.patch(CLOCK, return_value=float(argument["now"])):
            response = httpx.Response(200, json=argument["body"])
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


def file_delete_shape(argument: DeleteCase) -> Object:
    with mock.patch.dict(os.environ, argument.get("environment", {}), clear=True):
        params: Object = {"api_base": argument.get("params_api_base")}
        retrieve: list[Json] = [
            request_view(RETRIEVE_REQUEST, file_id, params)
            for file_id in argument["file_ids"]
        ]
        delete: list[Json] = [
            request_view(DELETE_REQUEST, file_id, params)
            for file_id in argument["file_ids"]
        ]
        responses: list[Json] = []
        for body in argument["bodies"]:
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
    return {"retrieve": retrieve, "delete": delete, "responses": responses}


def file_content_bytes(argument: ContentCase) -> Object:
    with mock.patch.dict(os.environ, argument.get("environment", {}), clear=True):
        urls: list[Json] = []
        for file_id in argument["file_ids"]:
            url, query = cast(
                "tuple[str, dict[str, str]]",
                CONTENT_REQUEST(
                    file_content_request={"file_id": file_id},
                    optional_params={},
                    litellm_params={"api_base": argument.get("params_api_base")},
                ),
            )
            urls.append({"url": url, "query": cast(Json, pairs_view(query))})
        response = httpx.Response(
            argument["status"],
            content=bytes(argument["body"]),
            headers=[(name, value) for name, value in argument["headers"]],
        )
        binary = cast(
            BinaryView,
            CONTENT_RESPONSE(
                raw_response=response,
                logging_obj=cast(Logging, None),
                litellm_params={},
            ),
        )
        result: Object = {
            "status": binary.response.status_code,
            "headers": cast(Json, dict(binary.response.headers)),
            "body": cast(Json, list(binary.response.content)),
        }
        error_spec = argument["error"]
        error = cast(
            ErrorView,
            GET_ERROR_CLASS(
                error_spec["message"],
                error_spec["status"],
                pairs_object(error_spec["headers"]),
            ),
        )
    return {"urls": urls, "result": result, "error": error_view(error)}


def file_beta_header(argument: BetaHeaderCase) -> Object:
    scenarios: list[Json] = []
    for scenario in argument["scenarios"]:
        with mock.patch.dict(os.environ, scenario.get("environment", {}), clear=True):
            params: Object = {"api_base": scenario.get("params_api_base")}
            api_key = scenario.get("api_key")
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
                for base in scenario.get("url_bases", [])
            ]
            headers: dict[str, str] = pairs_object(
                scenario.get("caller_headers", [])
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
                    api_base=scenario.get("api_base"),
                )
            except ValueError as error:
                entry["raised"] = raised(error)
            error_spec = scenario["error"]
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
