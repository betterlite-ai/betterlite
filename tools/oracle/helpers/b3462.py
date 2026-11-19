import asyncio
import uuid
from typing import Final, Literal, TypeAlias, TypedDict
from uuid import UUID

from litellm.proxy.rag_endpoints.endpoints import parse_rag_ingest_request
from litellm.proxy.rag_endpoints.upload_security import EicarTestMalwareScanner
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Message, Scope

BOUNDARY: Final = "betterlite-oracle-boundary"
FIXED_UUID: Final = UUID("550e8400-e29b-41d4-a716-446655440000")
ParsedRequest: TypeAlias = tuple[
    dict[str, JsonValue], tuple[str, bytes, str] | None, str | None, str | None
]
PARSED: Final[TypeAdapter[ParsedRequest]] = TypeAdapter(ParsedRequest)


class TextPart(TypedDict):
    name: str
    kind: Literal["text"]
    text: str


class UploadPart(TypedDict):
    name: str
    kind: Literal["upload"]
    filename: str
    content_hex: str
    content_type: str


class MultipartInput(TypedDict):
    mode: Literal["multipart"]
    parts: list[TextPart | UploadPart]


class JsonInput(TypedDict):
    mode: Literal["json"]
    body: str


Input = MultipartInput | JsonInput


class FileOutput(TypedDict):
    filename: str
    content_hex: str
    content_type: str


class Output(TypedDict):
    ingest_options: dict[str, JsonValue]
    file: FileOutput | None
    file_url: str | None
    file_id: str | None


class BodyChannel:
    def __init__(self, body: bytes) -> None:
        self.body = body

    async def __call__(self) -> Message:
        message: Message = {"type": "http.request", "body": self.body, "more_body": False}
        return message


class FixedUuid:
    def __call__(self) -> UUID:
        return FIXED_UUID


def part_header(part: TextPart | UploadPart) -> bytes:
    name = part["name"]
    match part["kind"]:
        case "text":
            return f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode()
        case "upload":
            filename = part["filename"]
            media_type = part["content_type"]
            return (
                f'Content-Disposition: form-data; name="{name}"; filename="{filename}"\r\n'
                f"Content-Type: {media_type}\r\n\r\n"
            ).encode()


def part_content(part: TextPart | UploadPart) -> bytes:
    match part["kind"]:
        case "text":
            return part["text"].encode()
        case "upload":
            return bytes.fromhex(part["content_hex"])


def multipart_body(parts: list[TextPart | UploadPart]) -> bytes:
    sections: list[bytes] = []
    for part in parts:
        sections.append(f"--{BOUNDARY}\r\n".encode())
        sections.append(part_header(part))
        sections.append(part_content(part))
        sections.append(b"\r\n")
    sections.append(f"--{BOUNDARY}--\r\n".encode())
    return b"".join(sections)


def build_request(content_type: str, body: bytes) -> Request:
    headers: list[tuple[bytes, bytes]] = [(b"content-type", content_type.encode())]
    scope: Scope = {
        "type": "http",
        "method": "POST",
        "path": "/v1/rag/ingest",
        "query_string": b"",
        "headers": headers,
    }
    return Request(scope, BodyChannel(body))


async def decode(content_type: str, body: bytes) -> Output:
    request = build_request(content_type, body)
    parsed = PARSED.validate_python(await parse_rag_ingest_request(request, EicarTestMalwareScanner()))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/rag_endpoints/endpoints.py#L404-L525
    ingest_options, file_data, file_url, file_id = parsed
    file: FileOutput | None = None
    if file_data is not None:
        filename, content, media_type = file_data
        file = {"filename": filename, "content_hex": content.hex(), "content_type": media_type}
    return {
        "ingest_options": ingest_options,
        "file": file,
        "file_url": file_url,
        "file_id": file_id,
    }


def run(input_value: Input) -> Output:
    uuid.uuid4 = FixedUuid()
    match input_value["mode"]:
        case "multipart":
            content_type = f"multipart/form-data; boundary={BOUNDARY}"
            return asyncio.run(decode(content_type, multipart_body(input_value["parts"])))
        case "json":
            return asyncio.run(decode("application/json", input_value["body"].encode()))
