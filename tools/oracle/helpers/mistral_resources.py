import json
import os
import time
from datetime import UTC, datetime
from functools import partial
from typing import Final, Literal, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.mistral.batches.transformation import MistralBatchesConfig
from litellm.llms.mistral.files.transformation import MistralFilesConfig
from litellm.types.llms.openai import CreateBatchRequest, CreateFileRequest
from pydantic import JsonValue, TypeAdapter


class UploadParts(TypedDict):
    file: tuple[str, bytes, str]
    purpose: tuple[None, str]


LOGGING: Final = Logging(
    model="",
    messages=None,
    stream=False,
    call_type="",
    start_time=datetime(2024, 1, 1, tzinfo=UTC),
    litellm_call_id="",
    function_id="",
)
FILES: Final = MistralFilesConfig.__new__(MistralFilesConfig)
BATCHES: Final = MistralBatchesConfig.__new__(MistralBatchesConfig)
JSON: Final = TypeAdapter(JsonValue)
UPLOAD_PARTS: Final = TypeAdapter(UploadParts)


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
    list_bodies: list[JsonValue]


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


def upload_outcome(upload: Upload, call: Files) -> JsonValue:
    params: dict[str, JsonValue] = {"api_base": call["api_base"]}
    url = FILES.get_complete_url(call["api_base"], call["api_key"], "", {}, params)
    file = (upload["filename"], bytes(upload["content"]), upload["content_type"])
    body = FILES.transform_create_file_request(
        "", CreateFileRequest(file=file, purpose=upload["purpose"]), {}, params  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L472
    )
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
    url, query = FILES.transform_list_files_request(purpose, {}, {"api_base": call["api_base"]})
    return {"ok": True, "url": url, "query": [[name, value] for name, value in query.items()]}


def file_response(body: JsonValue) -> JsonValue:
    response = httpx.Response(200, content=json.dumps(body).encode())
    file = FILES.transform_create_file_response(None, response, LOGGING, {})
    return JSON.validate_json(file.model_dump_json(exclude_none=True))


def list_response(body: JsonValue) -> JsonValue:
    response = httpx.Response(200, content=json.dumps(body).encode())
    files = FILES.transform_list_files_response(response, LOGGING, {})
    return [JSON.validate_json(file.model_dump_json(exclude_none=True)) for file in files]


def files_outcome(call: Files) -> JsonValue:
    time.time = partial(float, call["now"])
    return {
        "uploads": [upload_outcome(upload, call) for upload in call["uploads"]],
        "list": [list_outcome(purpose, call) for purpose in call["list_purposes"]],
        "bodies": [file_response(body) for body in call["bodies"]],
        "list_bodies": [list_response(body) for body in call["list_bodies"]],
    }


def create_outcome(call: Create) -> JsonValue:
    params: dict[str, JsonValue] = {"api_base": call["api_base"]}
    api_base, api_key, model, data = call["api_base"], call["api_key"], call["model"], call["data"]
    try:
        headers = BATCHES.validate_environment(
            headers={},
            model="",
            messages=(),
            optional_params={},
            litellm_params=params,
            api_key=api_key,
            api_base=api_base,
        )
        url = BATCHES.get_complete_batch_url(api_base, api_key, model, {}, params, data)
        body = BATCHES.transform_create_batch_request(model, data, {}, params)
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
    batch = BATCHES.transform_create_batch_response(None, response, LOGGING, {})
    return {"ok": True, "batch": JSON.validate_json(batch.model_dump_json())}


def retrieve_outcome(call: Retrieve) -> JsonValue:
    params: dict[str, JsonValue] = {"api_base": call["api_base"], "api_key": call["api_key"]}
    try:
        request = BATCHES.transform_retrieve_batch_request(call["batch_id"], {}, params)
    except ValueError:
        return {"ok": False, "status": None}
    plan = {
        "ok": True,
        "method": request["method"],
        "url": request["url"],
        "headers": request["headers"],
    }
    return JSON.validate_python(plan)


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
