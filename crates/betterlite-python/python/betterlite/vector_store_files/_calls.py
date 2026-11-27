import json
from collections.abc import Awaitable, Callable
from typing import Final, TypeAlias, TypeVar

from pydantic import TypeAdapter

from betterlite._endpoints import (
    CallValue,
    acall_native,
    call_native,
    read_settings,
    to_call_payload,
)
from betterlite._endpoints.payload import is_dropped
from betterlite._native import NativeVectorFileError, NativeVectorFileSession
from betterlite.types.vector_store_files import (
    VectorStoreFileContentResponse,
    VectorStoreFileDeleteResponse,
    VectorStoreFileListResponse,
    VectorStoreFileObject,
)

FILE: Final[TypeAdapter[VectorStoreFileObject]] = TypeAdapter(VectorStoreFileObject)
PAGE: Final[TypeAdapter[VectorStoreFileListResponse]] = TypeAdapter(
    VectorStoreFileListResponse
)
CONTENT: Final[TypeAdapter[VectorStoreFileContentResponse]] = TypeAdapter(
    VectorStoreFileContentResponse
)
DELETED: Final[TypeAdapter[VectorStoreFileDeleteResponse]] = TypeAdapter(
    VectorStoreFileDeleteResponse
)
SETTINGS: Final[tuple[str, ...]] = ("api_base", "api_key", "openai_key", "organization")
Answer = TypeVar("Answer")
SyncAnswer: TypeAlias = Callable[[dict[str, CallValue]], Answer]
AsyncAnswer: TypeAlias = Callable[[dict[str, CallValue]], Awaitable[Answer]]


# the call's own set fields, then its kwargs without the handles a Rust transport cannot take
def call_values(kwargs: dict[str, CallValue], **named: CallValue) -> dict[str, CallValue]:
    for name in tuple(kwargs):
        if is_dropped(name, kwargs[name]):
            kwargs.pop(name)
    kwargs.pop("litellm_logging_obj", None)
    present = {name: value for name, value in named.items() if value is not None}
    return {**present, "kwargs": kwargs}


# the payload holds the caller's own dict, so the flag popped here never reaches the wire
def dispatched(
    values: dict[str, CallValue],
    options: dict[str, CallValue],
    flag: str,
    asynchronous: AsyncAnswer[Answer],
    synchronous: SyncAnswer[Answer],
) -> Answer | Awaitable[Answer]:
    flagged = options.pop(flag, False) is True
    return asynchronous(values) if flagged else synchronous(values)


def create_now(values: dict[str, CallValue]) -> VectorStoreFileObject:
    answer = call_native(
        NativeVectorFileSession(json.dumps(read_settings(SETTINGS))).create,
        to_call_payload(values),
        NativeVectorFileError,
    )
    return FILE.validate_python(answer)


async def create_async(values: dict[str, CallValue]) -> VectorStoreFileObject:
    answer = await acall_native(
        NativeVectorFileSession(json.dumps(read_settings(SETTINGS))).acreate,
        to_call_payload(values),
        NativeVectorFileError,
    )
    return FILE.validate_python(answer)


def list_now(values: dict[str, CallValue]) -> VectorStoreFileListResponse:
    answer = call_native(
        NativeVectorFileSession(json.dumps(read_settings(SETTINGS))).list,
        to_call_payload(values),
        NativeVectorFileError,
    )
    return PAGE.validate_python(answer)


async def list_async(values: dict[str, CallValue]) -> VectorStoreFileListResponse:
    answer = await acall_native(
        NativeVectorFileSession(json.dumps(read_settings(SETTINGS))).alist,
        to_call_payload(values),
        NativeVectorFileError,
    )
    return PAGE.validate_python(answer)


def retrieve_now(values: dict[str, CallValue]) -> VectorStoreFileObject:
    answer = call_native(
        NativeVectorFileSession(json.dumps(read_settings(SETTINGS))).retrieve,
        to_call_payload(values),
        NativeVectorFileError,
    )
    return FILE.validate_python(answer)


async def retrieve_async(values: dict[str, CallValue]) -> VectorStoreFileObject:
    answer = await acall_native(
        NativeVectorFileSession(json.dumps(read_settings(SETTINGS))).aretrieve,
        to_call_payload(values),
        NativeVectorFileError,
    )
    return FILE.validate_python(answer)


def content_now(values: dict[str, CallValue]) -> VectorStoreFileContentResponse:
    answer = call_native(
        NativeVectorFileSession(json.dumps(read_settings(SETTINGS))).retrieve_content,
        to_call_payload(values),
        NativeVectorFileError,
    )
    return CONTENT.validate_python(answer)


async def content_async(values: dict[str, CallValue]) -> VectorStoreFileContentResponse:
    answer = await acall_native(
        NativeVectorFileSession(json.dumps(read_settings(SETTINGS))).aretrieve_content,
        to_call_payload(values),
        NativeVectorFileError,
    )
    return CONTENT.validate_python(answer)


def update_now(values: dict[str, CallValue]) -> VectorStoreFileObject:
    answer = call_native(
        NativeVectorFileSession(json.dumps(read_settings(SETTINGS))).update,
        to_call_payload(values),
        NativeVectorFileError,
    )
    return FILE.validate_python(answer)


async def update_async(values: dict[str, CallValue]) -> VectorStoreFileObject:
    answer = await acall_native(
        NativeVectorFileSession(json.dumps(read_settings(SETTINGS))).aupdate,
        to_call_payload(values),
        NativeVectorFileError,
    )
    return FILE.validate_python(answer)


def delete_now(values: dict[str, CallValue]) -> VectorStoreFileDeleteResponse:
    answer = call_native(
        NativeVectorFileSession(json.dumps(read_settings(SETTINGS))).delete,
        to_call_payload(values),
        NativeVectorFileError,
    )
    return DELETED.validate_python(answer)


async def delete_async(values: dict[str, CallValue]) -> VectorStoreFileDeleteResponse:
    answer = await acall_native(
        NativeVectorFileSession(json.dumps(read_settings(SETTINGS))).adelete,
        to_call_payload(values),
        NativeVectorFileError,
    )
    return DELETED.validate_python(answer)
