import base64
import io
from typing import Final, Literal, TypedDict

from litellm.proxy._types import ProxyException
from litellm.proxy.openai_files_endpoints.batch_file_validation import (
    BATCH_LINE_SHAPE,
    PASSTHROUGH_BATCH_LINE_SHAPE,
    BatchFileValidationFailure,
    BatchLineShape,
    check_batch_file_upload,
    raise_batch_file_validation_failure,
)
from litellm.proxy.openai_files_endpoints.general_upload_validation import (
    UploadValidationFailure,
    check_allowed_extension,
    check_blocked_extension,
    check_unsafe_filename,
    check_upload_file_size,
    coerce_optional_int_setting,
    coerce_optional_str_list_setting,
    raise_upload_validation_failure,
)
from pydantic import JsonValue, TypeAdapter, ValidationError

BATCH_ADAPTER: Final[TypeAdapter[int | None]] = TypeAdapter(int | None)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class BytesSource(TypedDict):
    kind: Literal["bytes"]
    data: str
    repeat: int
    pad: int


class SeekableSource(TypedDict):
    kind: Literal["seekable"]
    data: str
    repeat: int
    pad: int
    position: int


Source = BytesSource | SeekableSource


class UnsafeFilenameCall(TypedDict):
    function: Literal["unsafe_filename"]
    filename: str | None


class ExtensionsCall(TypedDict):
    function: Literal["extensions"]
    filename: str | None
    allowed: list[str] | None
    blocked: list[str] | None


class UploadSizeCall(TypedDict):
    function: Literal["upload_size"]
    source: Source
    max_file_size_mb: int | None


class BatchCall(TypedDict):
    function: Literal["batch"]
    filename: str | None
    source: Source
    max_batch_file_size_mb: int | None
    line_shape: Literal["openai", "vertex"]


class SettingsCall(TypedDict):
    function: Literal["settings"]
    max_file_size_mb: JsonValue
    max_batch_file_size_mb: JsonValue
    allowed_file_extensions: JsonValue
    blocked_file_extensions: JsonValue


Call = UnsafeFilenameCall | ExtensionsCall | UploadSizeCall | BatchCall | SettingsCall


class Input(TypedDict):
    calls: list[Call]


class CoercedSettings(TypedDict):
    max_file_size_mb: int | None
    max_batch_file_size_mb: int | None
    allowed_file_extensions: list[str] | None
    blocked_file_extensions: list[str] | None


class Failure(TypedDict):
    message: str
    type: str
    param: str | None
    code: str
    status: int


def exception_answer(error: ProxyException) -> Failure:
    return {
        "message": error.message,
        "type": error.type,
        "param": error.param,
        "code": error.code,
        "status": int(error.code),
    }


def upload_answer(failure: UploadValidationFailure | None) -> Failure | None:
    if failure is None:
        return None
    try:
        raise_upload_validation_failure(failure)
    except ProxyException as error:
        return exception_answer(error)


def batch_answer(failure: BatchFileValidationFailure | None) -> Failure | None:
    if failure is None:
        return None
    try:
        raise_batch_file_validation_failure(failure)
    except ProxyException as error:
        return exception_answer(error)


def source_bytes(source: Source) -> bytes:
    data = base64.b64decode(source["data"]) * source["repeat"]
    return data + b" " * source["pad"]


def string_items(value: tuple[str, ...] | None) -> list[str] | None:
    if value is None:
        return None
    return list(value)


def settings_answer(call: SettingsCall) -> JsonValue:
    try:
        max_file_size_mb = coerce_optional_int_setting(call["max_file_size_mb"])
        max_batch_file_size_mb = BATCH_ADAPTER.validate_python(call["max_batch_file_size_mb"])
        allowed = coerce_optional_str_list_setting(call["allowed_file_extensions"])
        blocked = coerce_optional_str_list_setting(call["blocked_file_extensions"])
    except (TypeError, ValidationError):
        return JSON.validate_python({"error": "invalid_setting"})
    settings: CoercedSettings = {
        "max_file_size_mb": max_file_size_mb,
        "max_batch_file_size_mb": max_batch_file_size_mb,
        "allowed_file_extensions": string_items(allowed),
        "blocked_file_extensions": string_items(blocked),
    }
    return JSON.validate_python({"settings": settings})


def extensions_answer(call: ExtensionsCall) -> JsonValue:
    allowed = None if call["allowed"] is None else tuple(call["allowed"])
    blocked = None if call["blocked"] is None else tuple(call["blocked"])
    return JSON.validate_python(
        {
            "allowed": upload_answer(check_allowed_extension(call["filename"], allowed)),
            "blocked": upload_answer(check_blocked_extension(call["filename"], blocked)),
        }
    )


def size_answer(call: UploadSizeCall) -> JsonValue:
    source = call["source"]
    limit = call["max_file_size_mb"]
    if source["kind"] == "bytes":
        failure = check_upload_file_size(source_bytes(source), limit)
        return JSON.validate_python({"failure": upload_answer(failure), "position": None})
    buffer = io.BytesIO(source_bytes(source))
    buffer.seek(source["position"])
    failure = check_upload_file_size(buffer, limit)
    return JSON.validate_python({"failure": upload_answer(failure), "position": buffer.tell()})


def batch_shape(line_shape: Literal["openai", "vertex"]) -> BatchLineShape:
    if line_shape == "openai":
        return BATCH_LINE_SHAPE
    return PASSTHROUGH_BATCH_LINE_SHAPE


def batch_call_answer(call: BatchCall) -> JsonValue:
    source = call["source"]
    shape = batch_shape(call["line_shape"])
    limit = call["max_batch_file_size_mb"]
    if source["kind"] == "bytes":
        failure = check_batch_file_upload(call["filename"], source_bytes(source), limit, shape)
        return JSON.validate_python({"failure": batch_answer(failure), "position": None})
    buffer = io.BytesIO(source_bytes(source))
    buffer.seek(source["position"])
    failure = check_batch_file_upload(call["filename"], buffer, limit, shape)
    return JSON.validate_python({"failure": batch_answer(failure), "position": buffer.tell()})


def answer(call: Call) -> JsonValue:
    if call["function"] == "unsafe_filename":
        failure = upload_answer(check_unsafe_filename(call["filename"]))
        return JSON.validate_python({"failure": failure})
    if call["function"] == "extensions":
        return extensions_answer(call)
    if call["function"] == "upload_size":
        return size_answer(call)
    if call["function"] == "batch":
        return batch_call_answer(call)
    return settings_answer(call)


def run(input_value: Input) -> JsonValue:
    return [answer(call) for call in input_value["calls"]]
