import os
import time
from collections.abc import Callable
from datetime import UTC, datetime
from functools import partial
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.gemini.files.transformation import GoogleAIStudioFilesHandler
from litellm.types.llms.openai import CreateFileRequest
from litellm.types.llms.openai import OpenAIFileObject as FileObject
from openai.types.file_deleted import FileDeleted
from pydantic import JsonValue, TypeAdapter

Json: TypeAlias = JsonValue
Object: TypeAlias = dict[str, Json]
Headers: TypeAlias = dict[str, str]
FileValue: TypeAlias = (
    bytes
    | tuple[str | None, bytes]
    | tuple[str | None, bytes, str | None]
    | tuple[str | None, bytes, str | None, Headers]
)
PlanTransform: TypeAlias = Callable[[str, CreateFileRequest, Object, Object], Object]
UrlTransform: TypeAlias = Callable[[str, Object, Object], tuple[str, Headers]]
CreateReplyTransform: TypeAlias = Callable[
    [str | None, httpx.Response, Logging, Object], FileObject
]
ReplyTransform: TypeAlias = Callable[[httpx.Response, Logging, Object], FileObject]
DeleteTransform: TypeAlias = Callable[[httpx.Response, Logging, Object], FileDeleted]


class RequestCall(TypedDict):
    function: Literal["supported", "environment", "upload_url"]
    api_key: NotRequired[str]
    headers: NotRequired[Headers]
    environment: NotRequired[Headers]
    api_base: NotRequired[str]
    params: NotRequired[Object]


class FileInput(TypedDict):
    data: list[int]
    filename: NotRequired[str]
    content_type: NotRequired[str]
    headers: NotRequired[Headers]


class CreatePlan(TypedDict):
    function: Literal["create_plan"]
    now: int
    file: FileInput


class Replies(TypedDict):
    function: Literal["create_reply", "retrieve_reply"]
    bodies: list[str]
    timezone: str


class Target(TypedDict):
    function: Literal["retrieve_target", "delete_target"]
    file_ids: list[str]
    params: NotRequired[Object]
    environment: NotRequired[Headers]


class DeleteReplyItem(TypedDict):
    status: int
    url: NotRequired[str]
    body: NotRequired[str]


class DeleteReply(TypedDict):
    function: Literal["delete_reply"]
    replies: list[DeleteReplyItem]


Call: TypeAlias = RequestCall | CreatePlan | Replies | Target | DeleteReply


class Input(TypedDict):
    calls: list[Call]


class InitialStep(TypedDict):
    headers: Headers
    data: Object


class UploadStep(TypedDict):
    headers: Headers
    data: bytes


class Plan(TypedDict):
    initial_request: InitialStep
    upload_request: UploadStep


INPUT = TypeAdapter(Input)
PLAN = TypeAdapter(Plan)
HANDLER: Final = GoogleAIStudioFilesHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L55
COMPLETE_URL: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L83
    "Callable[[str | None, str | None, str, Object, Object, bool | None], str]",
    HANDLER.get_complete_url,
)
CREATE_PLAN: Final = cast(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L124
    "PlanTransform", HANDLER.transform_create_file_request
)
CREATE_RESPONSE: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L176
    "CreateReplyTransform", HANDLER.transform_create_file_response
)
RETRIEVE_REQUEST: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L214
    "UrlTransform", HANDLER.transform_retrieve_file_request
)
RETRIEVE_RESPONSE: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L274
    "ReplyTransform", HANDLER.transform_retrieve_file_response
)
DELETE_REQUEST: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L318
    "UrlTransform", HANDLER.transform_delete_file_request
)
DELETE_RESPONSE: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L355
    "DeleteTransform", HANDLER.transform_delete_file_response
)
LOGGING: Final = Logging(
    model="gemini-2.5-flash",
    messages=[],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L576
    stream=False,
    call_type="completion",
    start_time=datetime(2026, 1, 1, tzinfo=UTC),
    litellm_call_id="studio-files",
    function_id="studio-files",
)


def environment_of(call: RequestCall | Target) -> Headers:
    given = call.get("environment")
    return {} if given is None else given


def params_of(call: RequestCall | Target) -> Object:
    given = call.get("params")
    return {} if given is None else given


def create_file(file: FileInput) -> FileValue:
    data = bytes(file["data"])
    filename = file.get("filename")
    content_type = file.get("content_type")
    headers = file.get("headers")
    if headers is not None:
        return (filename, data, content_type, headers)
    if content_type is not None:
        return (filename, data, content_type)
    if filename is not None:
        return (filename, data)
    return data


def environment(call: RequestCall) -> Object:
    headers = call.get("headers")
    caller: Headers = {} if headers is None else headers
    os.environ.clear()
    os.environ.update(environment_of(call))
    resolved = HANDLER.validate_environment(
        headers=caller,
        model="",
        messages=[],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params={},
        litellm_params={},
        api_key=call.get("api_key"),
        api_base=None,
    )
    return {"headers": dict(resolved)}


def upload_url(call: RequestCall) -> Object:
    os.environ.clear()
    os.environ.update(environment_of(call))
    url = COMPLETE_URL(
        call.get("api_base"), call.get("api_key"), "", {}, params_of(call), None
    )
    return {"url": url}


def create_plan(call: CreatePlan) -> Object:
    saved = time.time
    time.time = partial(float, call["now"])
    try:
        plan = PLAN.validate_python(
            CREATE_PLAN(
                "", {"file": create_file(call["file"]), "purpose": "batch"}, {}, {}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L472
            )
        )
    finally:
        time.time = saved
    return {
        "headers": dict(plan["initial_request"]["headers"]),
        "data": plan["initial_request"]["data"],
        "upload_headers": dict(plan["upload_request"]["headers"]),
        "upload_data": list(plan["upload_request"]["data"]),
    }


def file_view(file: FileObject) -> Object:
    return {
        "id": file.id,
        "bytes": file.bytes,
        "created_at": file.created_at,
        "filename": file.filename,
        "object": file.object,
        "purpose": file.purpose,
        "status": file.status,
        "status_details": file.status_details,
    }


def replies(call: Replies) -> Object:
    os.environ["TZ"] = call["timezone"]
    time.tzset()
    files: list[Json] = []
    for body in call["bodies"]:
        response = httpx.Response(200, content=body.encode("utf-8"))
        if call["function"] == "create_reply":
            file = CREATE_RESPONSE(None, response, LOGGING, {})
        else:
            file = RETRIEVE_RESPONSE(response, LOGGING, {})
        files.append(file_view(file))
    return {"files": files}


def target_view(pair: tuple[str, Headers]) -> Object:
    url, query = pair
    return {"url": url, "query": [[name, value] for name, value in query.items()]}


def targets(call: Target) -> Object:
    os.environ.clear()
    os.environ.update(environment_of(call))
    params = params_of(call)
    transform = RETRIEVE_REQUEST if call["function"] == "retrieve_target" else DELETE_REQUEST
    views: list[Json] = [
        target_view(transform(file_id, {}, params)) for file_id in call["file_ids"]
    ]
    return {"targets": views}


def delete_reply(call: DeleteReply) -> Object:
    replies: list[Json] = []
    for item in call["replies"]:
        body = item.get("body")
        content = b"" if body is None else body.encode("utf-8")
        url = item.get("url")
        request = None if url is None else httpx.Request("DELETE", url)
        response = httpx.Response(item["status"], content=content, request=request)
        deleted = DELETE_RESPONSE(response, LOGGING, {})
        replies.append({"id": deleted.id, "object": deleted.object, "deleted": deleted.deleted})
    return {"replies": replies}


def resolve_request(call: Call) -> Json:
    if call["function"] == "supported":
        params: list[Json] = list(HANDLER.get_supported_openai_params(""))
        return {"supported": params}
    if call["function"] == "environment":
        return environment(call)
    if call["function"] == "upload_url":
        return upload_url(call)
    if call["function"] == "create_plan":
        return create_plan(call)
    raise ValueError(f"unknown studio_files function: {call['function']}")


def resolve_reply(call: Call) -> Json:
    if call["function"] == "create_reply" or call["function"] == "retrieve_reply":
        return replies(call)
    if call["function"] == "retrieve_target" or call["function"] == "delete_target":
        return targets(call)
    if call["function"] == "delete_reply":
        return delete_reply(call)
    raise ValueError(f"unknown studio_files function: {call['function']}")


def resolve(call: Call) -> Json:
    if call["function"] in {
        "create_reply",
        "retrieve_reply",
        "retrieve_target",
        "delete_target",
        "delete_reply",
    }:
        return resolve_reply(call)
    return resolve_request(call)


def run(input_value: Json) -> Json:
    parsed = INPUT.validate_python(input_value)
    return {"calls": [resolve(call) for call in parsed["calls"]]}
