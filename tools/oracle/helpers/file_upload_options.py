import asyncio
import json
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.openai_files_endpoints.common_utils import (
    extract_file_creation_params,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/common_utils.py#L918-L928
    validate_managed_files_requirement,
    validate_managed_id_requirement,
)
from pydantic import BaseModel, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from starlette.requests import Request
from starlette.types import Message, Scope

BOUNDARY: Final = "file-upload-options-boundary"
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class TextPart(TypedDict):
    kind: Literal["text"]
    name: str
    text: str


class UploadPart(TypedDict):
    kind: Literal["upload"]
    name: str
    filename: str


Part: TypeAlias = TextPart | UploadPart


class ExtractCall(TypedDict):
    function: Literal["extract"]
    request_body: NotRequired[dict[str, JsonValue] | None]
    body: NotRequired[dict[str, JsonValue] | None]
    query_params: NotRequired[dict[str, str] | None]
    headers: NotRequired[dict[str, str] | None]
    form_parts: NotRequired[list[Part] | None]
    target_model_names_form: NotRequired[str | None]
    target_storage_form: NotRequired[str | None]


class ManagedUploadCall(TypedDict):
    function: Literal["managed_upload"]
    target_model_names: list[str]
    model: str | None
    require_managed_files: JsonValue


class ManagedIdCall(TypedDict):
    function: Literal["managed_id"]
    resource_id: str | None
    resource_kind: Literal["file", "batch", "fine-tuning job"]
    require_managed_files: JsonValue


Call: TypeAlias = ExtractCall | ManagedUploadCall | ManagedIdCall


class Input(TypedDict):
    calls: list[Call]


class Rejection(BaseModel):
    status_code: int
    detail: str


class BodyReceiver:
    def __init__(self, body: bytes) -> None:
        self.body = body
        self.sent = False

    async def __call__(self) -> Message:
        if self.sent:
            return {"type": "http.disconnect"}
        self.sent = True
        return {"type": "http.request", "body": self.body, "more_body": False}


def text_part(part: TextPart) -> bytes:
    head = f'Content-Disposition: form-data; name="{part["name"]}"\r\n\r\n'
    return head.encode() + part["text"].encode() + b"\r\n"


def upload_part(part: UploadPart) -> bytes:
    disposition = (
        f'Content-Disposition: form-data; name="{part["name"]}"; '
        f'filename="{part["filename"]}"\r\n'
        "Content-Type: application/octet-stream\r\n\r\n"
    )
    return disposition.encode() + b"payload\r\n"


def part_bytes(part: Part) -> bytes:
    if part["kind"] == "upload":
        return upload_part(part)
    return text_part(part)


def multipart_body(parts: list[Part]) -> bytes:
    encoded = [f"--{BOUNDARY}\r\n".encode() + part_bytes(part) for part in parts]
    return b"".join(encoded) + f"--{BOUNDARY}--\r\n".encode()


def body_bytes(call: ExtractCall) -> bytes:
    parts = call.get("form_parts")
    if parts:
        return multipart_body(parts)
    body = call.get("body")
    if body is None:
        return b""
    return json.dumps(body).encode()


def content_type(call: ExtractCall) -> str:
    if call.get("form_parts"):
        return f"multipart/form-data; boundary={BOUNDARY}"
    return "application/json"


def header_pairs(
    headers: dict[str, str] | None, media_type: str
) -> list[tuple[bytes, bytes]]:
    pairs = [("content-type", media_type)]
    if headers is not None:
        pairs.extend(headers.items())
    return [(name.lower().encode(), value.encode()) for name, value in pairs]


def query_string(query_params: dict[str, str] | None) -> bytes:
    if query_params is None:
        return b""
    joined = "&".join(f"{name}={value}" for name, value in query_params.items())
    return joined.encode()


def build_request(call: ExtractCall) -> Request:
    scope: Scope = {
        "type": "http",
        "method": "POST",
        "path": "/v1/files",
        "query_string": query_string(call.get("query_params")),
        "headers": header_pairs(call.get("headers"), content_type(call)),
    }
    return Request(scope, BodyReceiver(body_bytes(call)))


def extract_answer(call: ExtractCall) -> dict[str, JsonValue]:
    request = build_request(call)
    params = asyncio.run(
        extract_file_creation_params(
            request=request,
            request_body=call.get("request_body"),
            target_model_names_form=call.get("target_model_names_form"),
            target_storage_form=call.get("target_storage_form"),
        )
    )
    names: list[JsonValue] = [*params.target_model_names]
    return {
        "target_storage": params.target_storage,
        "target_model_names": names,
        "model": params.model,
    }


def rejection_answer(error: HTTPException) -> dict[str, JsonValue]:
    rejection = Rejection.model_validate(error, from_attributes=True)
    return {"status": rejection.status_code, "detail": rejection.detail}


def managed_upload_answer(call: ManagedUploadCall) -> dict[str, JsonValue]:
    litellm.require_managed_files = call["require_managed_files"] is True
    try:
        validate_managed_files_requirement(call["target_model_names"], call["model"])
    except HTTPException as error:
        return rejection_answer(error)
    return {"ok": True}


def managed_id_answer(call: ManagedIdCall) -> dict[str, JsonValue]:
    litellm.require_managed_files = call["require_managed_files"] is True
    try:
        asyncio.run(
            validate_managed_id_requirement(
                resource_id=call["resource_id"],
                resource_kind=call["resource_kind"],
                user_api_key_dict=UserAPIKeyAuth(),
                managed_files_obj=None,
            )
        )
    except HTTPException as error:
        return rejection_answer(error)
    return {"ok": True}


def answer(call: Call) -> JsonValue:
    match call["function"]:
        case "extract":
            return extract_answer(call)
        case "managed_upload":
            return managed_upload_answer(call)
        case "managed_id":
            return managed_id_answer(call)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
