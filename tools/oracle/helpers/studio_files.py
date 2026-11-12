import os
import time
from collections.abc import Callable
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
    function: Literal["replies"]
    shape: Literal["create", "retrieve"]
    bodies: list[str]
    timezone: str


class Targets(TypedDict):
    function: Literal["targets"]
    file_ids: list[str]
    delete: NotRequired[bool]
    api_key: NotRequired[str]
    api_base: NotRequired[str]
    environment: NotRequired[Headers]


class DeleteReplyItem(TypedDict):
    status: int
    url: NotRequired[str]
    body: NotRequired[str]


class DeleteReply(TypedDict):
    function: Literal["delete_reply"]
    replies: list[DeleteReplyItem]


Call: TypeAlias = CreatePlan | Replies | Targets | DeleteReply


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
CREATE_PLAN: Final = cast(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L124
    "Callable[[str, CreateFileRequest, Object, Object], Object]",
    HANDLER.transform_create_file_request,
)
CREATE_RESPONSE: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L176
    "Callable[[str | None, httpx.Response, Logging, Object], FileObject]",
    HANDLER.transform_create_file_response,
)
RETRIEVE_RESPONSE: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L274
    "Callable[[httpx.Response, Logging, Object], FileObject]",
    HANDLER.transform_retrieve_file_response,
)
DELETE_RESPONSE: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L355
    "Callable[[httpx.Response, Logging, Object], FileDeleted]",
    HANDLER.transform_delete_file_response,
)
RETRIEVE_REQUEST: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L214
    "Callable[[str, Object, Object], tuple[str, Headers]]",
    HANDLER.transform_retrieve_file_request,
)
DELETE_REQUEST: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L318
    "Callable[[str, Object, Object], tuple[str, Headers]]",
    HANDLER.transform_delete_file_request,
)
LOGGING: Final = cast("Logging", None)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/files/transformation.py#L24


def file_value(file: FileInput) -> FileValue:
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


def create_plan(call: CreatePlan) -> Json:
    saved = time.time
    time.time = partial(float, call["now"])
    try:
        plan = PLAN.validate_python(
            CREATE_PLAN("", {"file": file_value(call["file"]), "purpose": "batch"}, {}, {})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L472
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


def replies(call: Replies) -> Json:
    os.environ["TZ"] = call["timezone"]
    time.tzset()
    files: list[Json] = []
    for body in call["bodies"]:
        response = httpx.Response(200, content=body.encode("utf-8"))
        if call["shape"] == "create":
            file = CREATE_RESPONSE(None, response, LOGGING, {})
        else:
            file = RETRIEVE_RESPONSE(response, LOGGING, {})
        files.append(file_view(file))
    return {"files": files}


def targets(call: Targets) -> Json:
    given = call.get("environment")
    os.environ.clear()
    os.environ.update({} if given is None else given)
    params: Object = {"api_key": call.get("api_key"), "api_base": call.get("api_base")}
    transform = DELETE_REQUEST if call.get("delete") is True else RETRIEVE_REQUEST
    views: list[Json] = []
    for file_id in call["file_ids"]:
        url, query = transform(file_id, {}, params)
        views.append({"url": url, "query": [[name, value] for name, value in query.items()]})
    return {"targets": views}


def delete_reply(call: DeleteReply) -> Json:
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


def resolve(call: Call) -> Json:
    if call["function"] == "create_plan":
        return create_plan(call)
    if call["function"] == "replies":
        return replies(call)
    if call["function"] == "targets":
        return targets(call)
    return delete_reply(call)


def run(input_value: Json) -> Json:
    parsed = INPUT.validate_python(input_value)
    return {"calls": [resolve(call) for call in parsed["calls"]]}
