import asyncio
import logging
import math
from collections.abc import Mapping
from io import BytesIO
from typing import Annotated, Final, Literal, NotRequired, Required, TypeAlias, TypedDict, cast
from urllib.parse import urlencode

from fastapi import UploadFile as FastApiUploadFile
from litellm.proxy.common_utils.http_parsing_utils import (
    check_file_size_under_limit,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L390-L450
    coerce_numeric_form_fields,
    convert_upload_files_to_file_data,
    extract_nested_form_metadata,
    get_form_data,
    get_request_body,
    numeric_form_fields,
)
from pydantic import JsonValue
from starlette.datastructures import Headers
from starlette.datastructures import UploadFile as StarletteUploadFile
from starlette.requests import Request
from starlette.types import Message, Scope
from typing_extensions import ReadOnly, override

BOUNDARY: Final = "BetterliteFormBoundary"
WARNING_KINDS: Final = (
    ("Skipping UploadFile in metadata extraction for key: ", "upload_skipped"),
    ("Invalid metadata key format (empty path): ", "empty_path"),
    (
        "Cannot create nested path - intermediate value is not a dict at: ",
        "intermediate_not_object",
    ),
    ("Cannot set value - parent is not a dict for key: ", "parent_not_object"),
)

# numeric_form_fields reads these annotation objects exactly as get_type_hints hands them over
ANNOTATIONS = {
    "int": int,
    "float": float,
    "optional_int": int | None,
    "optional_float": float | None,
    "annotated_int": Annotated[int, "meta"],
    "annotated_optional_float": Annotated[float | None, "meta"],
    "required_int": Required[int],
    "notrequired_int": NotRequired[int],
    "readonly_int": ReadOnly[int],
    "bool": bool,
    "str": str,
    "literal": Literal[3],
    "list": list[int],
    "union": int | float,
    "other": dict[str, int],
}


class Part(TypedDict):
    kind: Literal["text", "upload"]
    name: str
    value: str
    filename: str
    content_type: str | None
    content: str


class Operation(TypedDict):
    operation: Literal[
        "form_data",
        "numeric_form_coercion",
        "nested_form_metadata",
        "upload_size_unset",
        "request_body_dispatch",
    ]
    parts: list[Part]
    fields: list[tuple[str, JsonValue]]
    annotations: dict[str, str]
    prefix: str | None
    file_size: int | None
    existing_metadata: dict[str, JsonValue]
    method: str
    content_type: str
    body: str


UploadedFile: TypeAlias = tuple[str, bytes, str | None]
MemberValue: TypeAlias = JsonValue | StarletteUploadFile | UploadedFile
FormMember: TypeAlias = MemberValue | list[MemberValue] | list[UploadedFile]


class Replay:
    def __init__(self, body: bytes) -> None:
        self.body = body
        self.ended = False

    async def __call__(self) -> Message:
        if self.ended:
            return {"type": "http.disconnect"}
        self.ended = True
        return {"type": "http.request", "body": self.body, "more_body": False}


class FormWarnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.entries: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        for prefix, kind in WARNING_KINDS:
            if message.startswith(prefix):
                self.entries.append({"kind": kind, "subject": message.removeprefix(prefix)})
                break


def encode_parts(parts: list[Part]) -> tuple[bytes, bytes]:
    if all(part["kind"] == "text" for part in parts):
        pairs = [(part["name"], part["value"]) for part in parts]
        return urlencode(pairs).encode(), b"application/x-www-form-urlencoded"
    chunks: list[bytes] = []
    for part in parts:
        line = f'Content-Disposition: form-data; name="{part["name"]}"'
        if part["kind"] == "upload":
            line = f'{line}; filename="{part["filename"]}"'
            content_type = part["content_type"]
            if content_type is not None:
                line = f"{line}\r\nContent-Type: {content_type}"
        chunks.append(f"--{BOUNDARY}\r\n{line}\r\n\r\n".encode())
        content = part["content"] if part["kind"] == "upload" else part["value"]
        chunks.append(content.encode())
        chunks.append(b"\r\n")
    chunks.append(f"--{BOUNDARY}--\r\n".encode())
    return b"".join(chunks), f"multipart/form-data; boundary={BOUNDARY}".encode()


def form_map(parts: list[Part]) -> dict[str, str | StarletteUploadFile]:
    form: dict[str, str | StarletteUploadFile] = {}
    for part in parts:
        if part["kind"] == "upload":
            form[part["name"]] = upload_file(part)
        else:
            form[part["name"]] = part["value"]
    return form


def upload_file(part: Part) -> FastApiUploadFile:
    content = part["content"].encode()
    content_type = part["content_type"]
    headers = None if content_type is None else Headers({"content-type": content_type})
    return FastApiUploadFile(
        file=BytesIO(content),
        size=len(content),
        filename=part["filename"],
        headers=headers,
    )


def wire_float(number: float) -> JsonValue:
    if math.isnan(number):
        return "nan"
    if math.isinf(number):
        return "positive_infinity" if number > 0.0 else "negative_infinity"
    return number


async def wire(value: FormMember) -> JsonValue:
    if value is None:
        return None
    wired: JsonValue = None
    match value:
        case StarletteUploadFile() as upload:
            content = await upload.read()
            wired = {
                "filename": upload.filename,
                "content": content.decode(),
                "content_type": upload.content_type,
                "size": upload.size,
            }
        case tuple() as file:
            wired = [file[0], file[1].decode(), file[2]]
        case list() as items:
            wired = [await wire(item) for item in items]
        case dict() as entries:
            wired = {key: await wire(item) for key, item in entries.items()}
        case float() as number:
            wired = wire_float(number)
        case bool() as flag:
            wired = flag
        case int() as number:
            wired = number
        case str() as text:
            wired = text
    return wired


async def form_data_call(call: Operation) -> JsonValue:
    body, content_type = encode_parts(call["parts"])
    headers: list[tuple[bytes, bytes]] = [(b"content-type", content_type)]
    scope: Scope = {"type": "http", "method": "POST", "path": "/", "headers": headers}
    form = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L453-L467
        "dict[str, str | StarletteUploadFile]",
        await get_form_data(Request(scope, Replay(body))),  # type: ignore[misc]  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L453-L467
    )
    converted = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L470-L512
        "dict[str, FormMember]",
        await convert_upload_files_to_file_data(form),  # type: ignore[misc]  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L470-L512
    )
    result: dict[str, JsonValue] = {}
    for name, member in converted.items():
        result[name] = await wire(member)
    return result


async def metadata_call(call: Operation) -> JsonValue:
    form = form_map(call["parts"])
    warnings = FormWarnings()
    logger = logging.getLogger("LiteLLM Proxy")
    logger.addHandler(warnings)
    prefix = call.get("prefix")
    try:
        if prefix is None:
            extracted = extract_nested_form_metadata(form)
        else:
            extracted = extract_nested_form_metadata(form, prefix)
    finally:
        logger.removeHandler(warnings)
    metadata = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L530-L615
        "dict[str, JsonValue]", extracted
    )
    result: dict[str, JsonValue] = {
        "metadata": await wire(metadata),
        "warnings": warnings.entries,
    }
    return result


async def numeric_call(call: Operation) -> JsonValue:
    fields: dict[str, JsonValue] = dict(call["fields"])
    annotations = {name: ANNOTATIONS[kind] for name, kind in call["annotations"].items()}
    numeric = numeric_form_fields(annotations)
    coerced = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L143-L157
        "Mapping[str, JsonValue]",
        coerce_numeric_form_fields(fields, numeric),
    )
    result: dict[str, JsonValue] = {}
    for name, value in coerced.items():
        result[name] = await wire(value)
    return result


async def upload_size_call(call: Operation) -> JsonValue:
    size = call.get("file_size")
    content = b"A" * size if size is not None else b""
    upload = FastApiUploadFile(file=BytesIO(content), size=size, filename="audio.wav")
    request_data: dict[str, JsonValue] = {"model": "whisper"}
    existing = call.get("existing_metadata")
    if existing:
        request_data["metadata"] = dict(existing)
    proceeded = check_file_size_under_limit(request_data, upload, [])
    result: dict[str, JsonValue] = {"proceeded": proceeded, "metadata": request_data["metadata"]}
    return result


async def dispatch_call(call: Operation) -> JsonValue:
    headers: list[tuple[bytes, bytes]] = [(b"content-type", call["content_type"].encode())]
    scope: Scope = {"type": "http", "method": call["method"], "path": "/", "headers": headers}
    body = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L515-L527
        "dict[str, FormMember]",
        await get_request_body(Request(scope, Replay(call["body"].encode()))),  # type: ignore[misc]  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L515-L527
    )
    result: dict[str, JsonValue] = {}
    for name, member in body.items():
        result[name] = await wire(member)
    return result


def run(input_value: Operation) -> JsonValue:
    match input_value["operation"]:
        case "form_data":
            return asyncio.run(form_data_call(input_value))
        case "numeric_form_coercion":
            return asyncio.run(numeric_call(input_value))
        case "nested_form_metadata":
            return asyncio.run(metadata_call(input_value))
        case "upload_size_unset":
            return asyncio.run(upload_size_call(input_value))
        case "request_body_dispatch":
            return asyncio.run(dispatch_call(input_value))
