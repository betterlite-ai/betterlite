import base64
import io
from typing import Final, Literal, TypedDict

from litellm.proxy._types import ProxyException
from litellm.proxy.openai_files_endpoints.batch_file_validation import (
    BATCH_LINE_SHAPE,
    PASSTHROUGH_BATCH_LINE_SHAPE,
    BatchFileEmpty,
    BatchFileInvalidJsonLine,
    BatchFileLineNotObject,
    BatchFileMissingLineKey,
    BatchFileTooLarge,
    BatchFileValidationFailure,
    BatchFileWrongExtension,
    BatchLineShape,
    check_batch_file_upload,
    raise_batch_file_validation_failure,
)
from litellm.proxy.openai_files_endpoints.general_upload_validation import (
    UploadedFileBlockedExtension,
    UploadedFileExtensionNotAllowed,
    UploadedFileTooLarge,
    UploadedFileUnsafeFilename,
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
Failure = UploadValidationFailure | BatchFileValidationFailure


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


def source_bytes(source: Source) -> bytes:
    data = base64.b64decode(source["data"]) * source["repeat"]
    return data + b" " * source["pad"]


def exception_answer(error: ProxyException) -> dict[str, JsonValue]:
    return {
        "message": error.message,
        "type": error.type,
        "param": error.param,
        "code": error.code,
        "status": int(error.code),
    }


def raised_answer(failure: Failure | None) -> JsonValue:
    if failure is None:
        return None
    try:
        match failure:
            case (
                UploadedFileTooLarge()
                | UploadedFileExtensionNotAllowed()
                | UploadedFileBlockedExtension()
                | UploadedFileUnsafeFilename()
            ):
                raise_upload_validation_failure(failure)
            case (
                BatchFileTooLarge()
                | BatchFileWrongExtension()
                | BatchFileEmpty()
                | BatchFileInvalidJsonLine()
                | BatchFileLineNotObject()
                | BatchFileMissingLineKey()
            ):
                raise_batch_file_validation_failure(failure)
    except ProxyException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        return exception_answer(error)


def strings_or_none(value: tuple[str, ...] | None) -> list[JsonValue] | None:
    if value is None:
        return None
    return list(value)


def settings_answer(call: SettingsCall) -> dict[str, JsonValue]:
    try:
        size = coerce_optional_int_setting(call["max_file_size_mb"])
        batch = BATCH_ADAPTER.validate_python(call["max_batch_file_size_mb"])
        allowed = coerce_optional_str_list_setting(call["allowed_file_extensions"])
        blocked = coerce_optional_str_list_setting(call["blocked_file_extensions"])
    except (TypeError, ValidationError):
        return {"error": "invalid_setting"}
    return {
        "settings": {
            "max_file_size_mb": size,
            "max_batch_file_size_mb": batch,
            "allowed_file_extensions": strings_or_none(allowed),
            "blocked_file_extensions": strings_or_none(blocked),
        }
    }


def extensions_answer(call: ExtensionsCall) -> dict[str, JsonValue]:
    allowed = None if call["allowed"] is None else tuple(call["allowed"])
    blocked = None if call["blocked"] is None else tuple(call["blocked"])
    return {
        "allowed": raised_answer(check_allowed_extension(call["filename"], allowed)),
        "blocked": raised_answer(check_blocked_extension(call["filename"], blocked)),
    }


def size_answer(call: UploadSizeCall) -> dict[str, JsonValue]:
    source = call["source"]
    limit = call["max_file_size_mb"]
    if source["kind"] == "bytes":
        failure = check_upload_file_size(source_bytes(source), limit)
        answer = raised_answer(failure)
        return {"failure": answer, "position": None}
    buffer = io.BytesIO(source_bytes(source))
    buffer.seek(source["position"])
    failure = check_upload_file_size(buffer, limit)
    answer = raised_answer(failure)
    return {"failure": answer, "position": buffer.tell()}


def batch_shape(line_shape: Literal["openai", "vertex"]) -> BatchLineShape:
    if line_shape == "openai":
        return BATCH_LINE_SHAPE
    return PASSTHROUGH_BATCH_LINE_SHAPE


def batch_call_answer(call: BatchCall) -> dict[str, JsonValue]:
    source = call["source"]
    shape = batch_shape(call["line_shape"])
    limit = call["max_batch_file_size_mb"]
    if source["kind"] == "bytes":
        failure = check_batch_file_upload(call["filename"], source_bytes(source), limit, shape)
        answer = raised_answer(failure)
        return {"failure": answer, "position": None}
    buffer = io.BytesIO(source_bytes(source))
    buffer.seek(source["position"])
    failure = check_batch_file_upload(call["filename"], buffer, limit, shape)
    answer = raised_answer(failure)
    return {"failure": answer, "position": buffer.tell()}


def answer(call: Call) -> JsonValue:
    if call["function"] == "unsafe_filename":
        failure = check_unsafe_filename(call["filename"])
        return {"failure": raised_answer(failure)}
    if call["function"] == "extensions":
        return extensions_answer(call)
    if call["function"] == "upload_size":
        return size_answer(call)
    if call["function"] == "batch":
        return batch_call_answer(call)
    return settings_answer(call)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
