import base64
import io
from typing import Final, TypedDict

import httpx
from litellm.types.utils import FileTypes
from pydantic import JsonValue
from typing_extensions import override

HEADER_NAMES: Final = ("authorization", "openai-organization")


class FileSpec(TypedDict):
    content_base64: str
    name: str | None


class NamedBuffer(io.BufferedReader):
    def __init__(self, raw: io.BytesIO, name: str) -> None:
        super().__init__(raw)
        self._name = name

    @property
    @override
    def name(self) -> str:
        return self._name


def file_value(spec: FileSpec | None) -> FileTypes | None:
    if spec is None:
        return None
    data = base64.b64decode(spec["content_base64"])
    name = spec["name"]
    if name is None:
        return data
    return NamedBuffer(io.BytesIO(data), name)


def files_value(specs: list[FileSpec | None] | None) -> list[FileTypes | None] | None:
    if specs is None:
        return None
    return [file_value(spec) for spec in specs]


class Part(TypedDict):
    name: str
    filename: str | None
    content_type: str | None
    content_base64: str


class Call(TypedDict):
    url: str
    headers: dict[str, str]
    data: dict[str, JsonValue]
    parts: list[Part]


def field_of(headers: str, marker: str) -> str | None:
    at = headers.find(marker)
    if at < 0:
        return None
    rest = headers[at + len(marker) :]
    end = rest.find('"')
    if end < 0:
        return rest
    return rest[:end]


def parts_of(request: httpx.Request) -> list[Part]:
    content_type = request.headers["content-type"]
    marker = "boundary="
    at = content_type.find(marker)
    if at < 0:
        raise ValueError("the image edit request is not multipart")
    delimiter = b"--" + content_type[at + len(marker) :].encode()
    parts: list[Part] = []
    for chunk in request.read().split(delimiter)[1:]:
        if chunk.startswith(b"--"):
            break
        head, _, data = chunk.partition(b"\r\n\r\n")
        fields = head.decode()
        name = field_of(fields, 'name="')
        if name is None:
            raise ValueError("a multipart part has no name")
        parts.append(
            Part(
                name=name,
                filename=field_of(fields, 'filename="'),
                content_type=field_of(fields, "Content-Type: "),
                content_base64=base64.b64encode(data.removesuffix(b"\r\n")).decode(),
            )
        )
    return parts


def call_value(request: httpx.Request) -> Call:
    data: dict[str, JsonValue] = {}
    files: list[Part] = []
    for part in parts_of(request):
        if part["filename"] is None:
            data[part["name"]] = base64.b64decode(part["content_base64"]).decode()
        else:
            files.append(part)
    headers = {name: request.headers[name] for name in HEADER_NAMES if name in request.headers}
    return Call(url=str(request.url), headers=headers, data=data, parts=files)
