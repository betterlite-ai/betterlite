import json
import os
import time
from collections.abc import Callable
from datetime import UTC, datetime
from functools import partial
from typing import Final, Literal, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.mistral.batches.transformation import MistralBatchesConfig
from litellm.llms.mistral.files.transformation import MistralFilesConfig
from litellm.types.llms.openai import CreateBatchRequest, CreateFileRequest
from pydantic import JsonValue, TypeAdapter

FileValue: TypeAlias = bytes | tuple[str | None, bytes] | tuple[str | None, bytes, str | None]
LOGGING: Final = Logging(
    model="",
    messages=[],
    stream=False,
    call_type="create_file",
    start_time=datetime.now(UTC),
    litellm_call_id="",
    function_id="",
)
FILES: Final = MistralFilesConfig()
BATCHES: Final = MistralBatchesConfig()
JSON: Final = TypeAdapter(JsonValue)


class UploadFile(TypedDict):
    filename: str | None
    content_type: str | None
    content: list[int]


class Upload(TypedDict):
    file: UploadFile
    purpose: Literal["assistants", "batch", "fine-tune", "messages"]


class FilesCall(TypedDict):
    api_base: str | None
    api_key: str | None
    now: int
    uploads: list[Upload]
    list_purposes: list[str | None]
    bodies: list[JsonValue]
    deleted: list[JsonValue]
    file_ids: list[str]


class FilesInput(TypedDict):
    operation: Literal["files"]
    env: dict[str, str]
    calls: list[FilesCall]


class BatchCreateCall(TypedDict):
    model: str
    api_base: str | None
    api_key: str | None
    data: CreateBatchRequest


class BatchCreateInput(TypedDict):
    operation: Literal["batch_create"]
    env: dict[str, str]
    calls: list[BatchCreateCall]


class BatchInput(TypedDict):
    operation: Literal["batch"]
    calls: list[JsonValue]


class RetrieveCall(TypedDict):
    batch_id: str
    api_base: str | None
    api_key: str | None


class RetrieveInput(TypedDict):
    operation: Literal["retrieve"]
    env: dict[str, str]
    calls: list[RetrieveCall]


Input: TypeAlias = FilesInput | BatchCreateInput | BatchInput | RetrieveInput


class UploadParts(TypedDict):
    file: tuple[str, bytes, str]
    purpose: tuple[None, str]


UPLOAD_PARTS = TypeAdapter(UploadParts)


def apply_environment(environment: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(environment)


def validated_headers(
    transform: Callable[..., dict[str, str]],
    api_key: str | None,
    api_base: str | None,
    litellm_params: dict[str, JsonValue],
) -> dict[str, str]:
    return transform(
        headers={},
        model="",
        messages=[],
        optional_params={},
        litellm_params=litellm_params,
        api_key=api_key,
        api_base=api_base,
    )


def file_value(upload: UploadFile) -> FileValue:
    content = bytes(upload["content"])
    content_type = upload["content_type"]
    filename = upload["filename"]
    if content_type is not None:
        return (filename, content, content_type)
    if filename is not None:
        return (filename, content)
    return content


def upload_outcome(
    upload: Upload, call: FilesCall, litellm_params: dict[str, JsonValue]
) -> JsonValue:
    url = FILES.get_complete_url(
        api_base=call["api_base"],
        api_key=call["api_key"],
        model="",
        optional_params={},
        litellm_params=litellm_params,
    )
    try:
        body = FILES.transform_create_file_request(
            model="",
            create_file_data=CreateFileRequest(
                file=file_value(upload["file"]), purpose=upload["purpose"]
            ),
            optional_params={},
            litellm_params=litellm_params,
        )
    except BaseLLMException as error:
        return {"ok": False, "status": error.status_code}
    parts = UPLOAD_PARTS.validate_python(body)
    filename, content, content_type = parts["file"]
    return {
        "ok": True,
        "url": url,
        "filename": filename,
        "content_type": content_type,
        "content": list(content),
        "purpose": parts["purpose"][1],
    }


def list_outcome(purpose: str | None, litellm_params: dict[str, JsonValue]) -> JsonValue:
    try:
        url, query = FILES.transform_list_files_request(
            purpose=purpose, optional_params={}, litellm_params=litellm_params
        )
    except BaseLLMException as error:
        return {"ok": False, "status": error.status_code}
    return {"ok": True, "url": url, "query": [[name, value] for name, value in query.items()]}


def request_outcome(
    transform: Callable[..., tuple[str, dict[str, str]]],
    file_id: str,
    litellm_params: dict[str, JsonValue],
) -> JsonValue:
    try:
        url, query = transform(
            file_id=file_id, optional_params={}, litellm_params=litellm_params
        )
    except ValueError:
        return {"ok": False, "status": None}
    return {"ok": True, "url": url, "query": [[name, value] for name, value in query.items()]}


def file_response(body: JsonValue, now: int) -> JsonValue:
    time.time = partial(float, now)
    response = httpx.Response(200, content=json.dumps(body).encode())
    file = FILES.transform_create_file_response(
        model=None, raw_response=response, logging_obj=LOGGING, litellm_params={}
    )
    return JSON.validate_json(file.model_dump_json())


def delete_response(body: JsonValue) -> JsonValue:
    response = httpx.Response(200, content=json.dumps(body).encode())
    deleted = FILES.transform_delete_file_response(
        raw_response=response, logging_obj=LOGGING, litellm_params={}
    )
    return JSON.validate_json(deleted.model_dump_json())


def files_call(call: FilesCall) -> JsonValue:
    litellm_params: dict[str, JsonValue] = {"api_base": call["api_base"]}
    try:
        headers = validated_headers(
            FILES.validate_environment, call["api_key"], call["api_base"], litellm_params
        )
    except ValueError:
        return {"ok": False, "status": None}
    time.time = partial(float, call["now"])
    return {
        "ok": True,
        "authorization": headers["Authorization"],
        "uploads": [upload_outcome(upload, call, litellm_params) for upload in call["uploads"]],
        "list": [list_outcome(purpose, litellm_params) for purpose in call["list_purposes"]],
        "bodies": [file_response(body, call["now"]) for body in call["bodies"]],
        "deleted": [delete_response(body) for body in call["deleted"]],
        "delete": [
            request_outcome(FILES.transform_delete_file_request, file_id, litellm_params)
            for file_id in call["file_ids"]
        ],
    }


def batch_create_call(call: BatchCreateCall) -> JsonValue:
    litellm_params: dict[str, JsonValue] = {"api_base": call["api_base"]}
    try:
        headers = validated_headers(
            BATCHES.validate_environment, call["api_key"], call["api_base"], litellm_params
        )
        url = BATCHES.get_complete_batch_url(
            api_base=call["api_base"],
            api_key=call["api_key"],
            model=call["model"],
            optional_params={},
            litellm_params=litellm_params,
            data=call["data"],
        )
        body = BATCHES.transform_create_batch_request(
            model=call["model"],
            create_batch_data=call["data"],
            optional_params={},
            litellm_params=litellm_params,
        )
    except ValueError:
        return {"ok": False, "status": None}
    return {
        "ok": True,
        "url": url,
        "authorization": headers["Authorization"],
        "body": JSON.validate_json(json.dumps(body)),
    }


def batch_call(batch: JsonValue) -> JsonValue:
    response = httpx.Response(200, content=json.dumps(batch).encode())
    job = BATCHES.transform_create_batch_response(
        model=None, raw_response=response, logging_obj=LOGGING, litellm_params={}
    )
    return {"ok": True, "batch": JSON.validate_json(job.model_dump_json())}


def retrieve_call(call: RetrieveCall) -> JsonValue:
    try:
        request = BATCHES.transform_retrieve_batch_request(
            batch_id=call["batch_id"],
            optional_params={},
            litellm_params={"api_base": call["api_base"], "api_key": call["api_key"]},
        )
    except ValueError:
        return {"ok": False, "status": None}
    return JSON.validate_python(
        {
            "ok": True,
            "method": request["method"],
            "url": request["url"],
            "headers": request["headers"],
        }
    )


def run(input_value: Input) -> JsonValue:
    match input_value["operation"]:
        case "files":
            apply_environment(input_value["env"])
            return {"calls": [files_call(call) for call in input_value["calls"]]}
        case "batch_create":
            apply_environment(input_value["env"])
            return {"calls": [batch_create_call(call) for call in input_value["calls"]]}
        case "batch":
            return {"calls": [batch_call(call["batch"]) for call in input_value["calls"]]}
        case "retrieve":
            apply_environment(input_value["env"])
            return {"calls": [retrieve_call(call) for call in input_value["calls"]]}
