import json
import os
import time
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


class Upload(TypedDict):
    filename: str | None
    content_type: str | None
    content: list[int]
    purpose: Literal["assistants", "batch", "fine-tune", "messages"]


class Files(TypedDict):
    api_base: str | None
    api_key: str | None
    now: int
    uploads: list[Upload]
    list_purposes: list[str | None]
    bodies: list[JsonValue]


class Create(TypedDict):
    api_base: str | None
    api_key: str | None
    model: str
    data: CreateBatchRequest


class Retrieve(TypedDict):
    batch_id: str
    api_base: str | None
    api_key: str | None


class Input(TypedDict):
    env: dict[str, str]
    files: Files | None
    creates: list[Create]
    jobs: list[JsonValue]
    retrieve: list[Retrieve]


class UploadParts(TypedDict):
    file: tuple[str, bytes, str]
    purpose: tuple[None, str]


UPLOAD_PARTS = TypeAdapter(UploadParts)


def file_value(upload: Upload) -> FileValue:
    content = bytes(upload["content"])
    content_type = upload["content_type"]
    filename = upload["filename"]
    if content_type is not None:
        return (filename, content, content_type)
    if filename is not None:
        return (filename, content)
    return content


def upload_outcome(upload: Upload, call: Files) -> JsonValue:
    params: dict[str, JsonValue] = {"api_base": call["api_base"]}
    url = FILES.get_complete_url(
        api_base=call["api_base"],
        api_key=call["api_key"],
        model="",
        optional_params={},
        litellm_params=params,
    )
    try:
        body = FILES.transform_create_file_request(
            model="",
            create_file_data=CreateFileRequest(
                file=file_value(upload), purpose=upload["purpose"]
            ),
            optional_params={},
            litellm_params=params,
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


def list_outcome(purpose: str | None, call: Files) -> JsonValue:
    try:
        url, query = FILES.transform_list_files_request(
            purpose=purpose,
            optional_params={},
            litellm_params={"api_base": call["api_base"]},
        )
    except BaseLLMException as error:
        return {"ok": False, "status": error.status_code}
    return {"ok": True, "url": url, "query": [[name, value] for name, value in query.items()]}


def file_response(body: JsonValue, now: int) -> JsonValue:
    time.time = partial(float, now)
    response = httpx.Response(200, content=json.dumps(body).encode())
    file = FILES.transform_create_file_response(
        model=None, raw_response=response, logging_obj=LOGGING, litellm_params={}
    )
    return JSON.validate_json(file.model_dump_json())


def files_outcome(call: Files) -> JsonValue:
    try:
        headers = FILES.validate_environment(
            headers={},
            model="",
            messages=[],
            optional_params={},
            litellm_params={"api_base": call["api_base"]},
            api_key=call["api_key"],
            api_base=call["api_base"],
        )
    except ValueError:
        return {"ok": False, "status": None}
    time.time = partial(float, call["now"])
    return {
        "ok": True,
        "authorization": headers["Authorization"],
        "uploads": [upload_outcome(upload, call) for upload in call["uploads"]],
        "list": [list_outcome(purpose, call) for purpose in call["list_purposes"]],
        "bodies": [file_response(body, call["now"]) for body in call["bodies"]],
    }


def create_outcome(call: Create) -> JsonValue:
    params: dict[str, JsonValue] = {"api_base": call["api_base"]}
    try:
        headers = BATCHES.validate_environment(
            headers={},
            model="",
            messages=[],
            optional_params={},
            litellm_params=params,
            api_key=call["api_key"],
            api_base=call["api_base"],
        )
        url = BATCHES.get_complete_batch_url(
            api_base=call["api_base"],
            api_key=call["api_key"],
            model=call["model"],
            optional_params={},
            litellm_params=params,
            data=call["data"],
        )
        body = BATCHES.transform_create_batch_request(
            model=call["model"],
            create_batch_data=call["data"],
            optional_params={},
            litellm_params=params,
        )
    except ValueError:
        return {"ok": False, "status": None}
    return {
        "ok": True,
        "url": url,
        "authorization": headers["Authorization"],
        "body": JSON.validate_json(json.dumps(body)),
    }


def job_outcome(job: JsonValue) -> JsonValue:
    response = httpx.Response(200, content=json.dumps(job).encode())
    batch = BATCHES.transform_create_batch_response(
        model=None, raw_response=response, logging_obj=LOGGING, litellm_params={}
    )
    return {"ok": True, "batch": JSON.validate_json(batch.model_dump_json())}


def retrieve_outcome(call: Retrieve) -> JsonValue:
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
    os.environ.clear()
    os.environ.update(input_value["env"])
    files = None if input_value["files"] is None else files_outcome(input_value["files"])
    return {
        "files": files,
        "create": [create_outcome(call) for call in input_value["creates"]],
        "jobs": [job_outcome(job) for job in input_value["jobs"]],
        "retrieve": [retrieve_outcome(call) for call in input_value["retrieve"]],
    }
