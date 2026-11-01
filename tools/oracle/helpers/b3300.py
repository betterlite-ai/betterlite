import asyncio
import base64
import io
import json
from typing import TypeAlias, cast

from litellm.proxy.ocr_endpoints.endpoints import (
    _parse_ocr_request,  # pyright: ignore[reportPrivateUsage]
)
from starlette.datastructures import FormData, Headers, UploadFile
from starlette.requests import Request

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_BOUNDARY = "OracleB3300Boundary"


def _build_request(content_type: str | None, header_req_format: str | None, body: bytes) -> Request:
    headers: list[tuple[bytes, bytes]] = []
    if content_type is not None:
        headers.append((b"content-type", content_type.encode()))
    if header_req_format is not None:
        headers.append((b"x-req-format", header_req_format.encode()))
    scope: dict[str, object] = {
        "type": "http",
        "method": "POST",
        "path": "/ocr",
        "headers": headers,
    }
    sent = False

    async def receive() -> dict[str, object]:
        nonlocal sent
        if sent:
            return {"type": "http.disconnect"}
        sent = True
        return {"type": "http.request", "body": body, "more_body": False}

    return Request(scope, receive)


def _file_bytes(multipart: dict[str, Json]) -> bytes:
    filler = multipart.get("file_filler_bytes")
    if filler is not None:
        return b"A" * cast(int, filler)
    encoded = multipart.get("file_base64")
    if encoded is None:
        return b""
    return base64.b64decode(cast(str, encoded))


def _multipart_body(multipart: dict[str, Json], file_content: bytes) -> bytes:
    filename = cast(str | None, multipart.get("file_name")) or "upload.bin"
    content_type = cast(str | None, multipart.get("file_content_type"))
    content_type_header = f"Content-Type: {content_type}\r\n" if content_type else ""
    parts = [
        f'--{_BOUNDARY}\r\nContent-Disposition: form-data; name="file"; filename="{filename}"\r\n'
        f"{content_type_header}\r\n".encode()
        + file_content
        + b"\r\n"
    ]
    for name, value in cast(list[list[str]], multipart.get("fields", [])):
        field = f'--{_BOUNDARY}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n{value}\r\n'
        parts.append(field.encode())
    parts.append(f"--{_BOUNDARY}--\r\n".encode())
    return b"".join(parts)


def _upload_file(filename: str | None, content_type: str | None, data: bytes) -> UploadFile:
    header_list = [(b"content-type", content_type.encode())] if content_type else []
    return UploadFile(io.BytesIO(data), filename=filename, headers=Headers(raw=header_list))


def _cached_form_request(
    multipart: dict[str, Json], header_req_format: str | None, file_content: bytes
) -> Request:
    filename = cast(str | None, multipart.get("file_name"))
    content_type = cast(str | None, multipart.get("file_content_type"))
    form_items: list[tuple[str, str | UploadFile]] = [
        ("file", _upload_file(filename, content_type, file_content))
    ]
    for name, value in cast(list[list[str]], multipart.get("fields", [])):
        form_items.append((name, value))
    request = _build_request("application/json", header_req_format, b"")
    # Simulates auth middleware's cached form.
    request._form = FormData(form_items)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return request


def _request_for(input_value: dict[str, Json]) -> Request:
    header_req_format = cast(str | None, input_value.get("header_req_format"))
    multipart = cast(dict[str, Json] | None, input_value.get("multipart"))
    if multipart is None:
        json_body = input_value.get("json_body")
        body = b"" if json_body is None else json.dumps(json_body).encode()
        return _build_request("application/json", header_req_format, body)
    file_content = _file_bytes(multipart)
    if cast(bool, input_value.get("use_cached_form", False)):
        return _cached_form_request(multipart, header_req_format, file_content)
    body = _multipart_body(multipart, file_content)
    content_type = f"multipart/form-data; boundary={_BOUNDARY}"
    return _build_request(content_type, header_req_format, body)


def _document_value(document_raw: dict[str, object]) -> Json:
    if document_raw.get("type") == "file":
        upload = cast(io.BytesIO, document_raw["file"])
        mime_type = cast("str | None", document_raw.get("mime_type"))
        return {
            "kind": "upload",
            "bytes_base64": base64.b64encode(upload.getvalue()).decode(),
            "mime_type": mime_type,
        }
    # document_raw is the caller's own JSON document object, so its values are Json-shaped.
    value = cast("dict[str, Json]", dict(document_raw))
    value["kind"] = "inline"
    return value


def _normalize(parsed: dict[str, object]) -> Json:
    fields = dict(parsed)
    req_format = cast("str | None", fields.pop("req_format", None))
    document_raw = fields.get("document")
    document_value: Json = None
    if isinstance(document_raw, dict):
        del fields["document"]
        document_value = _document_value(cast("dict[str, object]", document_raw))
    # The remaining fields are the caller's own JSON body or form values, so they are Json-shaped.
    typed_fields = cast("dict[str, Json]", fields)
    return {"document": document_value, "fields": typed_fields, "req_format": req_format}


def run(input_value: dict[str, Json]) -> Json:
    request = _request_for(input_value)
    parsed = asyncio.run(_parse_ocr_request(request))
    return _normalize(dict(parsed))
