import os
import time
from datetime import UTC, datetime
from functools import partial
from typing import Final, Literal, TypeAlias, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.xai.files.transformation import XAIFilesConfig
from pydantic import JsonValue, TypeAdapter

CLEARED: Final = ("XAI_API_KEY", "XAI_API_BASE")
CONFIG: Final = XAIFilesConfig.__new__(XAIFilesConfig)
LOG: Final = Logging(
    model="", messages=None, stream=False, call_type="", start_time=datetime.now(UTC),
    litellm_call_id="", function_id="",
)


class Reply(TypedDict):
    status: int
    body: str


class Raised(TypedDict):
    raised: str


class FileAnswer(TypedDict, total=False):
    id: str
    bytes: int
    created_at: int
    filename: str
    object: str
    purpose: str
    status: str
    expires_at: int


class DeletedAnswer(TypedDict):
    id: str
    object: str
    deleted: bool


class ContentAnswer(TypedDict):
    status: int
    headers: dict[str, str]
    body: list[int]


class ContentSpec(TypedDict):
    status: int
    headers: list[list[str]]
    body: list[int]


class Common(TypedDict):
    env: dict[str, str]
    xai_key: str | None


class UploadCall(TypedDict):
    api_base: str | None
    api_key: str | None
    now: int
    reply: Reply


class UploadInput(Common):
    operation: Literal["upload"]
    calls: list[UploadCall]


class FileCall(TypedDict):
    params_api_base: str | None
    file_id: str | None
    delete_reply: Reply
    content_reply: ContentSpec


class FilesInput(Common):
    operation: Literal["files"]
    calls: list[FileCall]


class ListCall(TypedDict):
    params_api_base: str | None
    purpose: str | None
    now: int
    reply: Reply


class ListInput(Common):
    operation: Literal["list"]
    calls: list[ListCall]


class UploadAnswer(TypedDict):
    supported: list[str]
    mapped: dict[str, JsonValue]
    url: str
    authorization: str | None
    file: FileAnswer | Raised


class FileEntry(TypedDict):
    delete: JsonValue
    content: JsonValue
    deleted: DeletedAnswer | Raised
    content_reply: ContentAnswer


class ListEntry(TypedDict):
    url: str
    next: JsonValue
    raised: str | None
    files: list[FileAnswer]
    raised_list: str | None


class UploadOutput(TypedDict):
    calls: list[UploadAnswer]


class FilesOutput(TypedDict):
    calls: list[FileEntry]


class ListOutput(TypedDict):
    calls: list[ListEntry]


Input: TypeAlias = UploadInput | FilesInput | ListInput
Output: TypeAlias = UploadOutput | FilesOutput | ListOutput
MAPPED: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
FILE_ANSWER: Final[TypeAdapter[FileAnswer]] = TypeAdapter(FileAnswer)
DELETED_ANSWER: Final[TypeAdapter[DeletedAnswer]] = TypeAdapter(DeletedAnswer)


def upload_call(call: UploadCall) -> UploadAnswer:
    time.time = partial(float, call["now"])
    supported: list[str] = [str(name) for name in CONFIG.get_supported_openai_params("")]
    response = httpx.Response(call["reply"]["status"], content=call["reply"]["body"].encode())
    file: FileAnswer | Raised
    try:
        created = CONFIG.transform_create_file_response(
            model=None, raw_response=response, logging_obj=LOG, litellm_params={}
        )
    except ValueError:
        file = {"raised": "reply"}
    else:
        file = FILE_ANSWER.validate_json(created.model_dump_json(exclude_none=True))
    headers = CONFIG.validate_environment(
        headers={},
        model="",
        messages=[],
        optional_params={},
        litellm_params={},
        api_key=call["api_key"],
        api_base=call["api_base"],
    )
    return {
        "supported": supported,
        "mapped": MAPPED.validate_python(
            CONFIG.map_openai_params(
                {"purpose": "assistants", "temperature": 2},
                {"purpose": "batch", "expires_after": {"anchor": "created_at"}},
                model="",
                drop_params=False,
            )
        ),
        "url": CONFIG.get_complete_url(
            api_base=call["api_base"],
            api_key=call["api_key"],
            model="",
            optional_params={},
            litellm_params={},
        ),
        "authorization": headers.get("Authorization"),
        "file": file,
    }


def file_target(
    params_api_base: str | None, file_id: str | None, *, content: bool
) -> JsonValue:
    litellm_params: dict[str, str | None] = {"api_base": params_api_base}
    try:
        if content:
            url, _query = CONFIG.transform_file_content_request(
                file_content_request={} if file_id is None else {"file_id": file_id},
                optional_params={},
                litellm_params=litellm_params,
            )
        else:
            url, _query = CONFIG.transform_retrieve_file_request(
                file_id="" if file_id is None else file_id,
                optional_params={},
                litellm_params=litellm_params,
            )
    except ValueError:
        return {"raised": "path"}
    return {"url": url}


def content_view(spec: ContentSpec) -> ContentAnswer:
    headers = [(name, value) for name, value in spec["headers"]]
    response = httpx.Response(spec["status"], content=bytes(spec["body"]), headers=headers)
    content = CONFIG.transform_file_content_response(
        raw_response=response, logging_obj=LOG, litellm_params={}
    )
    watched: dict[str, str] = {}
    for name in [name.lower() for name, _ in spec["headers"]]:
        watched[name] = content.response.headers[name]
    return {
        "status": content.response.status_code,
        "headers": watched,
        "body": list(content.response.content),
    }


def files_call(call: FileCall) -> FileEntry:
    reply = call["delete_reply"]
    response = httpx.Response(reply["status"], content=reply["body"].encode())
    deleted: DeletedAnswer | Raised
    try:
        removed = CONFIG.transform_delete_file_response(
            raw_response=response, logging_obj=LOG, litellm_params={}
        )
    except ValueError:
        deleted = {"raised": "reply"}
    else:
        deleted = DELETED_ANSWER.validate_json(removed.model_dump_json(exclude_none=True))
    return {
        "delete": file_target(call["params_api_base"], call["file_id"], content=False),
        "content": file_target(call["params_api_base"], call["file_id"], content=True),
        "deleted": deleted,
        "content_reply": content_view(call["content_reply"]),
    }


def list_call(call: ListCall) -> ListEntry:
    time.time = partial(float, call["now"])
    litellm_params: dict[str, str | None] = {"api_base": call["params_api_base"]}
    url, _query = CONFIG.transform_list_files_request(
        purpose=call["purpose"], optional_params={}, litellm_params=litellm_params
    )
    entry: ListEntry = {
        "url": url,
        "next": None,
        "raised": None,
        "files": [],
        "raised_list": None,
    }
    response = httpx.Response(call["reply"]["status"], content=call["reply"]["body"].encode())
    try:
        next_request = CONFIG.transform_list_files_next_request(
            raw_response=response, optional_params={}, litellm_params=litellm_params
        )
    except ValueError:
        entry["raised"] = "page"
    else:
        if next_request is not None:
            entry["next"] = {
                "url": next_request[0],
                "query": [[name, value] for name, value in next_request[1].items()],
            }
    try:
        files = CONFIG.transform_list_files_response(
            raw_response=response, logging_obj=LOG, litellm_params=litellm_params
        )
    except ValueError:
        entry["raised_list"] = "list"
    else:
        entry["files"] = [
            FILE_ANSWER.validate_json(item.model_dump_json(exclude_none=True)) for item in files
        ]
    return entry


def run(input_value: Input) -> Output:
    for name in CLEARED:
        os.environ.pop(name, None)
    os.environ.update(input_value["env"])
    litellm.xai_key = input_value["xai_key"]
    match input_value["operation"]:
        case "upload":
            return {"calls": [upload_call(call) for call in input_value["calls"]]}
        case "files":
            return {"calls": [files_call(call) for call in input_value["calls"]]}
        case "list":
            return {"calls": [list_call(call) for call in input_value["calls"]]}
