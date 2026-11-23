from collections.abc import Awaitable, Callable
from typing import Final, TypeAlias

from pydantic import JsonValue, TypeAdapter

from betterlite._completion.errors import raise_native_error
from betterlite._endpoints.payload import CallPayload
from betterlite._native import NativeError

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
NativeCall: TypeAlias = Callable[[str, list[bytes]], str]
AsyncNativeCall: TypeAlias = Callable[[str, list[bytes]], Awaitable[str]]
ErrorClass: TypeAlias = type[NativeError]


# The native method answers JSON text or raises its family's NativeError naming the class
def call_native(
    method: NativeCall,
    payload: CallPayload,
    error: ErrorClass = NativeError,
) -> JsonValue:
    answer: str
    try:
        answer = method(payload.text, payload.blobs)
    except error as failure:
        raise_native_error(failure)
    return JSON.validate_json(answer)


async def acall_native(
    method: AsyncNativeCall,
    payload: CallPayload,
    error: ErrorClass = NativeError,
) -> JsonValue:
    answer: str
    try:
        answer = await method(payload.text, payload.blobs)
    except error as failure:
        raise_native_error(failure)
    return JSON.validate_json(answer)
