import asyncio
from collections.abc import Awaitable, Callable
from typing import Final, NoReturn, TypeVar

import httpx2
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite._completion.errors import UnknownNativeError
from betterlite._native import NativeMcpError


class McpError(Exception):
    code: int | None
    message: str
    data: JsonValue

    def __init__(self, code: int | None, message: str, data: JsonValue = None) -> None:
        self.code = code
        self.message = message
        self.data = data
        super().__init__(message)


class UnsupportedMcpArgumentError(Exception):
    argument: str

    def __init__(self, argument: str) -> None:
        self.argument = argument
        super().__init__(f"{argument} is not supported by betterlite's MCP client")


UnsupportedMcpArgument = UnsupportedMcpArgumentError


class NativeMcpPayload(TypedDict):
    type: str
    message: str
    code: int | None
    data: JsonValue


class StatusDetails(TypedDict):
    status: int
    body: str
    url: str
    headers: list[tuple[str, str]]


PAYLOAD: Final[TypeAdapter[NativeMcpPayload]] = TypeAdapter(NativeMcpPayload)
STATUS_DETAILS: Final[TypeAdapter[StatusDetails]] = TypeAdapter(StatusDetails)

# the `type` each native error carries to the exception class LiteLLM's callers catch
ERROR_TYPES: Final[dict[str, type[BaseException]]] = {
    "McpError": McpError,
    "TimeoutError": TimeoutError,
    "RuntimeError": RuntimeError,
    "ValueError": ValueError,
    "CancelledError": asyncio.CancelledError,
    "HTTPStatusError": httpx2.HTTPStatusError,
}


# LiteLLM lets httpx2's HTTPStatusError through, so its request and response are rebuilt here
def _http_status_error(payload: NativeMcpPayload) -> httpx2.HTTPStatusError:
    details = STATUS_DETAILS.validate_python(payload["data"])
    request = httpx2.Request("POST", details["url"])
    response = httpx2.Response(
        details["status"],
        request=request,
        text=details["body"],
        headers=details["headers"],
    )
    return httpx2.HTTPStatusError(payload["message"], request=request, response=response)


def _build_error(payload: NativeMcpPayload, error_class: type[BaseException]) -> BaseException:
    if error_class is McpError:
        return McpError(payload["code"], payload["message"], payload["data"])
    if error_class is httpx2.HTTPStatusError:
        return _http_status_error(payload)
    return error_class(payload["message"])


def raise_native_error(failure: NativeMcpError) -> NoReturn:
    payload = PAYLOAD.validate_json(str(failure))
    error_class = ERROR_TYPES.get(payload["type"])
    if error_class is None:
        raise UnknownNativeError(str(failure)) from failure
    raise _build_error(payload, error_class) from failure


Answer = TypeVar("Answer")


# Every native call answers through here, so the payload of a failure becomes the right exception
async def native_answer(work: Awaitable[Answer]) -> Answer:
    try:
        return await work
    except NativeMcpError as failure:
        raise_native_error(failure)


# The pure transforms take JSON text and answer JSON text, and fail with the same payloads
def native_text(work: Callable[[str], str], text: str) -> str:
    try:
        return work(text)
    except NativeMcpError as failure:
        raise_native_error(failure)
