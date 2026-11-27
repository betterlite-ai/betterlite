import json
from collections.abc import Awaitable
from typing import Final, TypeAlias

from pydantic import TypeAdapter

from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints import (
    CallPayload,
    CallValue,
    acall_native,
    call_native,
    read_settings,
    to_call_payload,
)
from betterlite._endpoints.payload import LOGGER, LOGGER_FN_DROPPED, REFUSED
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
Stored: TypeAlias = (
    VectorStoreFileObject
    | VectorStoreFileListResponse
    | VectorStoreFileContentResponse
    | VectorStoreFileDeleteResponse
)


# A call answers at once or with a coroutine under its flag, so awaiting either gives the answer
async def settled(pending: Stored | Awaitable[Stored]) -> Stored:
    match pending:
        case Awaitable():
            return await pending
        case object():
            return pending


# LiteLLM's Python client and session cannot cross into the Rust transport, so they are named back
def refuse_handles(values: dict[str, CallValue]) -> None:
    for name in REFUSED:
        if values.pop(name, None) is not None:
            raise UnsupportedCallArgument(name)


# LiteLLM's logging object and logger function are bookkeeping a Rust call cannot use
def drop_bookkeeping(values: dict[str, CallValue]) -> None:
    values.pop("litellm_logging_obj", None)
    if values.pop("logger_fn", None) is not None:
        LOGGER.warning(LOGGER_FN_DROPPED)


def cleaned(kwargs: dict[str, CallValue]) -> dict[str, CallValue]:
    refuse_handles(kwargs)
    drop_bookkeeping(kwargs)
    return kwargs


# The call's own fields, then everything the caller left in **kwargs under the key the wire reads
def call_values(kwargs: dict[str, CallValue], **named: CallValue) -> dict[str, CallValue]:
    requested = cleaned(kwargs)
    return {**named, "kwargs": requested}


# A setting the caller left unset stays absent, so the fallback it would hide applies instead
def connection_values(**named: CallValue) -> dict[str, CallValue]:
    present: dict[str, CallValue] = {}
    for name, value in named.items():
        if value is not None:
            present[name] = value
    return present


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
