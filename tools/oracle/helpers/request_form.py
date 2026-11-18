import asyncio
import base64
import json
import math
from collections.abc import Mapping
from io import BytesIO
from typing import TypedDict
from urllib.parse import urlencode

import litellm
from litellm.proxy.common_utils.http_parsing_utils import (
    coerce_numeric_form_fields,
    check_file_size_under_limit,
    convert_upload_files_to_file_data,
    extract_nested_form_metadata,
    get_form_data,
    numeric_form_fields,
)
from pydantic import TypeAdapter
from starlette.datastructures import UploadFile
from starlette.requests import Request
from starlette.types import Message, Scope


class UploadFixture(TypedDict):
    name: str
    filename: str
    content_type: str
    content: str


class RequestFormInput(TypedDict):
    operation: str
    fields: list[list[str]]
    uploads: list[UploadFixture]
    annotations: dict[str, str]
    prefix: str
    file_size: int | None
    method: str
    content_type: str
    body: str


async def _fields(input_value: RequestFormInput) -> Mapping[str, object]:
    if input_value["uploads"]:
        boundary = "BetterliteRequestForm"
        chunks: list[bytes] = []
        for name, value in input_value["fields"]:
            chunks.append(
                f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n{value}\r\n'.encode()
            )
        for upload in input_value["uploads"]:
            chunks.append(
                (
                    f'--{boundary}\r\nContent-Disposition: form-data; name="{upload["name"]}"; '
                    f'filename="{upload["filename"]}"\r\nContent-Type: {upload["content_type"]}\r\n\r\n'
                ).encode()
                + upload["content"].encode()
                + b"\r\n"
            )
        chunks.append(f"--{boundary}--\r\n".encode())
        body = b"".join(chunks)
        content_type = f"multipart/form-data; boundary={boundary}".encode()
    else:
        body = urlencode(input_value["fields"]).encode()
        content_type = b"application/x-www-form-urlencoded"
    sent = False

    async def receive() -> Message:
        nonlocal sent
        if sent:
            return {"type": "http.disconnect"}
        sent = True
        return {"type": "http.request", "body": body, "more_body": False}

    scope: Scope = {
        "type": "http",
        "asgi": {"version": "3.0"},
        "http_version": "1.1",
        "method": "POST",
        "scheme": "http",
        "path": "/",
        "raw_path": b"/",
        "query_string": b"",
        "headers": [(b"content-type", content_type)],
        "client": ("127.0.0.1", 1),
        "server": ("localhost", 80),
    }
    return await get_form_data(Request(scope, receive))  # pyright: ignore[reportAny]  # LiteLLM annotates form values as Any: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L453-L467


def _normal(fields: dict[str, object]) -> dict[str, object]:
    normalized: dict[str, object] = {}
    for name, value in fields.items():
        if isinstance(value, float) and not math.isfinite(value):
            if math.isnan(value):
                normalized[name] = "nan"
            else:
                normalized[name] = "positive_infinity" if value > 0 else "negative_infinity"
        else:
            normalized[name] = value
    return normalized


def run(input_value: RequestFormInput) -> object:
    """Run the pinned LiteLLM form helpers for the selected request form operation."""
    _ = litellm
    operation = input_value["operation"]
    fields = asyncio.run(_fields(input_value))
    if operation == "numeric_form_coercion":
        annotations: dict[str, object] = {}
        for name, kind in input_value["annotations"].items():
            if kind == "int":
                annotations[name] = int
            elif kind == "float":
                annotations[name] = float
            elif kind == "optional_int":
                annotations[name] = int | None
            elif kind == "optional_float":
                annotations[name] = float | None
            else:
                annotations[name] = str
        return _normal(dict(coerce_numeric_form_fields(fields, numeric_form_fields(annotations))))
    if operation == "nested_form_metadata":
        return extract_nested_form_metadata(fields, input_value["prefix"])
    if input_value["uploads"]:
        asyncio.run(convert_upload_files_to_file_data(dict(fields)))
        upload = input_value["uploads"][0]
        encoded = base64.b64encode(upload["content"].encode()).decode()
        normalized = {
            name: value
            for name, value in fields.items()
            if not isinstance(value, UploadFile)
        }
        normalized[upload["name"]] = [[upload["filename"], encoded, upload["content_type"]]]
        return normalized
    if operation == "upload_size_unset":
        size = input_value["file_size"]
        file = UploadFile(
            BytesIO(b"A" * (size or 0)),
            filename="audio.wav",
            size=size,
        )
        request_data: dict[str, object] = {"model": "whisper"}
        proceeded = check_file_size_under_limit(request_data, file, [])
        metadata = request_data.get("metadata")
        if not isinstance(metadata, dict):
            return {"proceeded": proceeded, "file_size_in_mb": None}
        return {
            "proceeded": proceeded,
            "file_size_in_mb": metadata.get("file_size_in_mb"),
        }
    if operation == "request_body_dispatch":
        method = input_value["method"]
        if method != "POST":
            return {}
        content_type = input_value["content_type"]
        if content_type.startswith("application/json"):
            return TypeAdapter(dict[str, object]).validate_json(input_value["body"])
            return dict(fields)
    return _normal(fields)
