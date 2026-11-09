import asyncio
import base64
import json
import threading
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Final, TypeAlias, cast

import litellm
from litellm.llms.base_llm.ocr.transformation import OCRResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

BOUNDARY_MARKER: Final = "boundary="


@dataclass
class _Captured:
    method: str
    path: str
    headers: dict[str, str]
    body: bytes


@dataclass
class _Part:
    name: str
    filename: str | None
    content_type: str | None
    body: bytes


def _handler(
    replies: list[tuple[int, bytes]], captured: list[_Captured]
) -> type[BaseHTTPRequestHandler]:
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:
            length = int(self.headers.get("Content-Length", "0"))
            body = self.rfile.read(length) if length else b""
            captured.append(
                _Captured(
                    method=self.command,
                    path=self.path,
                    headers={name.lower(): value for name, value in self.headers.items()},
                    body=body,
                )
            )
            status, payload = replies.pop(0) if replies else (500, b"{}")
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    return Handler


def _start(
    replies: list[tuple[int, bytes]], captured: list[_Captured]
) -> ThreadingHTTPServer:
    server = ThreadingHTTPServer(("127.0.0.1", 0), _handler(replies, captured))
    thread = threading.Thread(
        target=server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True
    )
    thread.start()
    return server


def _base_url(server: ThreadingHTTPServer) -> str:
    host, port = cast("tuple[str, int]", server.server_address)
    return f"http://{host}:{port}"


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _optional_text(value: Json, name: str) -> str | None:
    if value is None:
        return None
    return _text(value, name)


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _reply(reply: dict[str, Json]) -> tuple[int, bytes]:
    status = reply.get("status", 200)
    if not isinstance(status, int) or isinstance(status, bool):
        raise ValueError("status must be an integer")
    return status, json.dumps(reply.get("body"), separators=(",", ":")).encode()


def _source(document: dict[str, Json]) -> str:
    for name in ("document_url", "image_url"):
        value = document.get(name)
        if isinstance(value, str):
            return value
    raise ValueError("the document needs a document_url or image_url")


async def _call(
    model: str,
    document: dict[str, Json],
    api_key: str | None,
    optional_params: dict[str, Json],
    base: str,
) -> OCRResponse:
    return await litellm.aocr(
        model=model,
        document=document,
        api_key=api_key,
        api_base=base,
        **optional_params,
    )


def _multipart_parts(content_type: str, body: bytes) -> list[_Part]:
    location = content_type.find(BOUNDARY_MARKER)
    if location < 0:
        raise ValueError("the multipart content-type carries no boundary")
    boundary = content_type[location + len(BOUNDARY_MARKER) :].split(";", maxsplit=1)[0].strip('"')
    delimiter = b"--" + boundary.encode()
    parts: list[_Part] = []
    for section in body.split(delimiter)[1:]:
        if section.startswith(b"--"):
            break
        stripped = section.removeprefix(b"\r\n")
        head, _, payload = stripped.partition(b"\r\n\r\n")
        parts.append(_part_from(head.decode(), payload.removesuffix(b"\r\n")))
    return parts


def _part_from(head: str, payload: bytes) -> _Part:
    name = ""
    filename: str | None = None
    content_type: str | None = None
    for line in head.split("\r\n"):
        field, _, value = line.partition(": ")
        lowered = field.lower()
        if lowered == "content-disposition":
            for attribute in value.split("; ")[1:]:
                key, _, quoted = attribute.partition("=")
                text = quoted.strip('"')
                if key == "name":
                    name = text
                elif key == "filename":
                    filename = text
        elif lowered == "content-type":
            content_type = value
    return _Part(name=name, filename=filename, content_type=content_type, body=payload)


def _part(part: _Part) -> Json:
    output: dict[str, Json] = {
        "name": part.name,
        "filename": part.filename,
        "content_type": part.content_type,
        "body_base64": base64.b64encode(part.body).decode(),
    }
    return output


def _observed(request: _Captured) -> Json:
    content_type = request.headers.get("content-type", "")
    output: dict[str, Json] = {
        "method": request.method,
        "path": request.path,
        "authorization": request.headers.get("authorization"),
        "content_type": content_type,
        "parts": None,
        "body": None,
    }
    if content_type.startswith("multipart/"):
        output["content_type"] = "multipart/form-data"
        output["parts"] = [_part(part) for part in _multipart_parts(content_type, request.body)]
    else:
        output["body"] = cast(Json, json.loads(request.body))
    return output


def _response(response: OCRResponse) -> Json:
    pages: list[Json] = []
    for page in response.pages:
        dumped = cast("dict[str, object]", page.model_dump())
        page_output: dict[str, Json] = {
            "index": page.index,
            "markdown": page.markdown,
            "blocks": cast(Json, dumped.get("blocks")),
        }
        pages.append(page_output)
    usage = response.usage_info
    usage_output: Json = None
    if usage is not None:
        usage_output = {"pages_processed": usage.pages_processed, "credits": usage.credits}
    output: dict[str, Json] = {
        "model": response.model,
        "pages": pages,
        "usage": usage_output,
    }
    return output


def run(input_value: dict[str, Json]) -> Json:
    model = _text(input_value.get("model"), "model")
    document = _object(input_value.get("document"), "document")
    api_key = _optional_text(input_value.get("api_key"), "api_key")
    optional_params = _object(input_value.get("optional_params", {}), "optional_params")
    upload = _object(input_value.get("upload"), "upload")
    parse = _object(input_value.get("parse"), "parse")
    source = _source(document)
    replies: list[tuple[int, bytes]] = []
    if source.startswith("reducto://"):
        replies.append(_reply(parse))
    elif source.startswith("data:"):
        replies.append(_reply(upload))
        replies.append(_reply(parse))
    captured: list[_Captured] = []
    server = _start(replies, captured)
    try:
        response = asyncio.run(
            _call(model, document, api_key, optional_params, _base_url(server))
        )
    finally:
        server.shutdown()
        server.server_close()
    output: dict[str, Json] = {
        "requests": [_observed(request) for request in captured],
        "response": _response(response),
    }
    return output
