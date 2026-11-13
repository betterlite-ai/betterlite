import os
import time
from datetime import UTC, datetime
from functools import partial
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.gemini.files.transformation import GoogleAIStudioFilesHandler
from litellm.types.llms.openai import CreateFileRequest, OpenAIFileObject
from openai.types.file_deleted import FileDeleted
from pydantic import JsonValue, TypeAdapter

Object: TypeAlias = dict[str, JsonValue]
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


class FilesApi(Protocol):
    def transform_create_file_request(
        self,
        model: str,
        create_file_data: CreateFileRequest,
        optional_params: Object,
        litellm_params: Object,
        /,
    ) -> Object: ...

    def transform_create_file_response(
        self,
        model: str | None,
        raw_response: httpx.Response,
        logging_obj: Logging,
        litellm_params: Object,
        /,
    ) -> OpenAIFileObject: ...

    def transform_delete_file_response(
        self, raw_response: httpx.Response, logging_obj: Logging, litellm_params: Object, /
    ) -> FileDeleted: ...

    def transform_delete_file_request(
        self, file_id: str, optional_params: Object, litellm_params: Object, /
    ) -> tuple[str, Headers]: ...


HANDLER: Final = GoogleAIStudioFilesHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L55
API: FilesApi = HANDLER


INPUT = TypeAdapter(Input)
PLAN = TypeAdapter(Plan)
TARGET = TypeAdapter(tuple[str, Headers])
MESSAGES: Final[list[str]] = []
LOGGING: Final = Logging(
    model="",
    messages=MESSAGES,
    stream=False,
    call_type="create_file",
    start_time=datetime.now(UTC),
    litellm_call_id="",
    function_id="",
)


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


def create_plan(call: CreatePlan) -> JsonValue:
    saved = time.time
    time.time = partial(float, call["now"])
    try:
        raw = API.transform_create_file_request(
            "",
            {"file": file_value(call["file"]), "purpose": "batch"},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L472
            {},
            {},
        )
    finally:
        time.time = saved
    plan = PLAN.validate_python(raw)
    return {
        "headers": dict(plan["initial_request"]["headers"]),
        "data": plan["initial_request"]["data"],
        "upload_headers": dict(plan["upload_request"]["headers"]),
        "upload_data": list(plan["upload_request"]["data"]),
    }


def replies(call: Replies) -> JsonValue:
    os.environ["TZ"] = call["timezone"]
    time.tzset()
    files: list[JsonValue] = []
    for body in call["bodies"]:
        response = httpx.Response(200, content=body.encode("utf-8"))
        if call["shape"] == "create":
            file = API.transform_create_file_response(None, response, LOGGING, {})
        else:
            file = HANDLER.transform_retrieve_file_response(response, LOGGING, {})  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L274
        files.append(
            {
                "id": file.id,
                "bytes": file.bytes,
                "created_at": file.created_at,
                "filename": file.filename,
                "object": file.object,
                "purpose": file.purpose,
                "status": file.status,
                "status_details": file.status_details,
            }
        )
    return {"files": files}


def targets(call: Targets) -> JsonValue:
    given = call.get("environment")
    os.environ.clear()
    os.environ.update({} if given is None else given)
    params: Object = {"api_key": call.get("api_key"), "api_base": call.get("api_base")}
    delete = call.get("delete") is True
    views: list[JsonValue] = []
    for file_id in call["file_ids"]:
        if delete:
            url, query = API.transform_delete_file_request(file_id, {}, params)
        else:
            url, query = TARGET.validate_python(
                HANDLER.transform_retrieve_file_request(file_id, {}, params)  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/files/transformation.py#L214
            )
        views.append({"url": url, "query": [[name, value] for name, value in query.items()]})
    return {"targets": views}


def delete_reply(call: DeleteReply) -> JsonValue:
    replies: list[JsonValue] = []
    for item in call["replies"]:
        body = item.get("body")
        content = b"" if body is None else body.encode("utf-8")
        url = item.get("url")
        request = None if url is None else httpx.Request("DELETE", url)
        response = httpx.Response(item["status"], content=content, request=request)
        deleted = API.transform_delete_file_response(response, LOGGING, {})
        replies.append({"id": deleted.id, "object": deleted.object, "deleted": deleted.deleted})
    return {"replies": replies}


def resolve(call: Call) -> JsonValue:
    if call["function"] == "create_plan":
        return create_plan(call)
    if call["function"] == "replies":
        return replies(call)
    if call["function"] == "targets":
        return targets(call)
    return delete_reply(call)


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    return {"calls": [resolve(call) for call in parsed["calls"]]}
