import os
import time
from datetime import UTC, datetime
from typing import (
    Final,
    Literal,
    NotRequired,
    Protocol,
    TypeAlias,
    TypedDict,
    runtime_checkable,
)

import httpx
from fastuuid import UUID
from litellm._uuid import uuid as litellm_uuid
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.vertex_ai.files.transformation import VertexAIFilesConfig
from litellm.types.llms.openai import OpenAIFileObject
from openai.types.file_deleted import FileDeleted
from pydantic import ConfigDict, JsonValue, TypeAdapter

Object: TypeAlias = dict[str, JsonValue]
Headers: TypeAlias = dict[str, str]
Purpose: TypeAlias = Literal["assistants", "batch", "fine-tune", "messages"]
Operation: TypeAlias = Literal[
    "object_name",
    "upload_url",
    "headers",
    "upload_reply",
    "retrieve_reply",
    "delete_reply",
    "list",
]
Shape: TypeAlias = Literal["create", "retrieve"]
FileValue: TypeAlias = bytes | tuple[str | None, bytes] | tuple[str | None, bytes, str]
ARBITRARY: ConfigDict = ConfigDict(arbitrary_types_allowed=True)


class FileCreate(TypedDict):
    file: FileValue
    purpose: Purpose


@runtime_checkable
class FilesApi(Protocol):
    def get_object_name(
        self,
        file_data: FileValue,
        purpose: str,
        deployment_model: str | None = None,
        *,
        passthrough: bool = False,
    ) -> str: ...

    def get_complete_file_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: Object,
        litellm_params: Object,
        data: FileCreate,
    ) -> str: ...

    def transform_create_file_response(
        self,
        model: str | None,
        raw_response: httpx.Response,
        logging_obj: Logging,
        litellm_params: Object,
    ) -> OpenAIFileObject: ...

    def transform_retrieve_file_response(  # ast-grep-ignore: no-long-names
        self,
        raw_response: httpx.Response,
        logging_obj: Logging,
        litellm_params: Object,
    ) -> OpenAIFileObject: ...

    def transform_delete_file_response(
        self,
        raw_response: httpx.Response,
        logging_obj: Logging,
        litellm_params: Object,
    ) -> FileDeleted: ...

    def transform_list_files_response(
        self,
        raw_response: httpx.Response,
        logging_obj: Logging,
        litellm_params: Object,
    ) -> list[OpenAIFileObject]: ...


class FileInput(TypedDict):
    content: str
    filename: NotRequired[str | None]
    content_type: NotRequired[str | None]


class Call(TypedDict):
    operation: Operation
    file: NotRequired[FileInput]
    purpose: NotRequired[Purpose]
    deployment_model: NotRequired[str | None]
    passthrough: NotRequired[bool]
    custom_endpoint: NotRequired[bool]
    bucket: NotRequired[str | None]
    api_base: NotRequired[str | None]
    uuid: NotRequired[int]
    headers: NotRequired[Headers]
    api_key: NotRequired[str]
    body: NotRequired[str]
    url: NotRequired[str | None]


class FixedUuid:
    value: int = 1

    def uuid4(self) -> UUID:
        pinned = self.value
        return UUID(int=pinned)


FIXED: Final = FixedUuid()
litellm_uuid.uuid4 = FIXED.uuid4
os.environ["TZ"] = "UTC"
time.tzset()
# LiteLLM's __init__ is untyped, and every method this file calls is stateless
HANDLER: Final = VertexAIFilesConfig.__new__(VertexAIFilesConfig)
# the protocol names these methods without the unknown LiteLLM parameter types behind them
API: FilesApi = TypeAdapter(FilesApi, config=ARBITRARY).validate_python(HANDLER)
PARAMS: Final[Object] = {}
MESSAGES: Final[list[JsonValue]] = []
LOGGING: Final = Logging(
    model="",
    messages=MESSAGES,
    stream=False,
    call_type="create_file",
    start_time=datetime(2024, 12, 4, 21, 53, 12, tzinfo=UTC),
    litellm_call_id="",
    function_id="",
)
INPUT: Final = TypeAdapter(Call)


def file_value(file: FileInput) -> FileValue:
    data = file["content"].encode("utf-8")
    filename = file.get("filename")
    content_type = file.get("content_type")
    if content_type is not None:
        return (filename, data, content_type)
    if filename is not None:
        return (filename, data)
    return data


def file_and_purpose(call: Call) -> tuple[FileInput, Purpose]:
    file = call.get("file")
    purpose = call.get("purpose")
    if file is None or purpose is None:
        raise ValueError("the call needs `file` and `purpose`")
    return file, purpose


def object_name(call: Call) -> JsonValue:
    file, purpose = file_and_purpose(call)
    name = API.get_object_name(
        file_value(file),
        purpose,
        deployment_model=call.get("deployment_model"),
        passthrough=call.get("passthrough") is True,
    )
    return {"name": name}


def upload_url(call: Call) -> JsonValue:
    file, purpose = file_and_purpose(call)
    params: Object = {
        "model": call.get("deployment_model"),
        "passthrough": call.get("passthrough") is True,
        "custom_endpoint": call.get("custom_endpoint") is True,
    }
    bucket = call.get("bucket")
    if bucket is not None:
        params["gcs_bucket_name"] = bucket
    data: FileCreate = {"file": file_value(file), "purpose": purpose}
    url = API.get_complete_file_url(call.get("api_base"), None, "", PARAMS, params, data)
    return {"url": url}


def environment_headers(call: Call) -> JsonValue:
    headers = call.get("headers")
    api_key = call.get("api_key")
    if headers is None or api_key is None:
        raise ValueError("the headers call needs `headers` and `api_key`")
    validated: Object = HANDLER.validate_environment(headers, "", [], PARAMS, PARAMS, api_key=api_key)  # type: ignore[misc]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
    return {"headers": validated}


def file_reply(call: Call, shape: Shape) -> JsonValue:
    body = call.get("body")
    if body is None:
        raise ValueError("the reply needs `body`")
    response = httpx.Response(200, content=body.encode("utf-8"))
    file = (
        API.transform_create_file_response(None, response, LOGGING, PARAMS)
        if shape == "create"
        else API.transform_retrieve_file_response(response, LOGGING, PARAMS)
    )
    return {
        "file": {
            "id": file.id,
            "bytes": file.bytes,
            "created_at": file.created_at,
            "filename": file.filename,
            "object": file.object,
            "purpose": file.purpose,
            "status": file.status,
            "expires_at": file.expires_at,
            "status_details": file.status_details,
        }
    }


def delete_reply(call: Call) -> JsonValue:
    url = call.get("url")
    request = None if url is None else httpx.Request("DELETE", url)
    response = httpx.Response(200, content=b"{}", request=request)
    deleted = API.transform_delete_file_response(response, LOGGING, PARAMS)
    return {"id": deleted.id, "object": deleted.object, "deleted": deleted.deleted}


def list_files() -> JsonValue:
    response = httpx.Response(200, content=b"{}")
    files = API.transform_list_files_response(response, LOGGING, PARAMS)
    return {"files": [file.id for file in files]}


def resolve(call: Call) -> JsonValue:
    operation = call["operation"]
    if operation == "object_name":
        return object_name(call)
    if operation == "upload_url":
        return upload_url(call)
    if operation == "headers":
        return environment_headers(call)
    if operation == "delete_reply":
        return delete_reply(call)
    if operation == "list":
        return list_files()
    shape: Shape = "create" if operation == "upload_reply" else "retrieve"
    return file_reply(call, shape)


def run(input_value: JsonValue) -> JsonValue:
    call = INPUT.validate_python(input_value)
    given = call.get("uuid")
    if given is not None:
        FIXED.value = given
    return resolve(call)
