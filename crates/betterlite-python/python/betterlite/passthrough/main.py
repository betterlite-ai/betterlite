from collections.abc import (
    AsyncIterable,
    AsyncIterator,
    Generator,
    Iterable,
    Iterator,
    Mapping,
    Sequence,
)
from http.cookiejar import CookieJar
from typing import Final, TypeAlias, TypedDict
from urllib.parse import parse_qsl

from pydantic import JsonValue, TypeAdapter

from betterlite._completion.arguments import SessionLike
from betterlite._completion.errors import NativeError, UnsupportedCallArgument, raise_native_error
from betterlite._endpoints.payload import CallValue, Readable, to_call_payload
from betterlite._native import (
    NativePassthroughError,
    NativePassthroughSession,
    NativePassthroughStream,
)
from betterlite.exceptions import APIError
from betterlite.exceptions._http import Headers, Response
from betterlite.types.llms.openai.core.aliases import FileTypes

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ERROR_STATUS: Final = 400

ScalarParam: TypeAlias = str | int | float | bool | None
ParamValue: TypeAlias = ScalarParam | Sequence[ScalarParam]
Content: TypeAlias = (
    str | bytes | bytearray | memoryview | Readable | Iterable[bytes] | AsyncIterable[bytes]
)
Files: TypeAlias = Mapping[str, FileTypes] | Sequence[tuple[str, FileTypes]]
Params: TypeAlias = str | bytes | Mapping[str, ParamValue] | Sequence[tuple[str, ParamValue]]
Cookies: TypeAlias = CookieJar | Mapping[str, str] | Sequence[tuple[str, str]]


class RouteValues(TypedDict):
    method: str
    endpoint: str
    model: str
    custom_llm_provider: str | None
    api_base: str | None
    api_key: str | None
    request_query_params: dict[str, JsonValue] | None
    request_headers: dict[str, str] | None
    content: bytes | None
    data: dict[str, CallValue] | None
    files: Files | None
    json: JsonValue
    params: Params | None
    cookies: Cookies | None
    client: SessionLike | None
    kwargs: Mapping[str, CallValue]


class PassthroughResponse(Response):
    def __init__(self, status_code: int, headers: Mapping[str, str], content: bytes) -> None:
        super().__init__(status_code=status_code, headers=headers, content=content)
        if "content-length" not in Headers(headers):
            self.headers.pop("content-length", None)

    def json(self) -> JsonValue:
        return JSON.validate_json(self.content)

    def raise_for_status(self) -> None:
        if self.status_code >= ERROR_STATUS:
            raise APIError(
                status_code=self.status_code,
                message=self.text,
                llm_provider="",
                model="",
                body=None,
            )


class PassthroughStreamingResponse:
    status_code: int
    headers: Headers
    _stream: NativePassthroughStream

    def __init__(
        self, status_code: int, headers: Mapping[str, str], stream: NativePassthroughStream
    ) -> None:
        self.status_code = status_code
        self.headers = Headers(headers)
        self._stream = stream

    def __iter__(self) -> Iterator[bytes]:
        return self

    def __next__(self) -> bytes:
        chunk = self._stream.next_chunk()
        if chunk is None:
            raise StopIteration
        return chunk

    # ast-grep-ignore: no-one-line-functions
    def close(self) -> None:
        self._stream.close()


# ast-grep-ignore: no-long-names
class AsyncPassthroughStreamingResponse:
    status_code: int
    headers: Headers
    _stream: NativePassthroughStream

    def __init__(
        self, status_code: int, headers: Mapping[str, str], stream: NativePassthroughStream
    ) -> None:
        self.status_code = status_code
        self.headers = Headers(headers)
        self._stream = stream

    def __await__(self) -> Generator[None, None, "AsyncPassthroughStreamingResponse"]:
        yield from ()
        return self

    def __aiter__(self) -> AsyncIterator[bytes]:
        return self

    async def __anext__(self) -> bytes:
        chunk = await self._stream.anext_chunk()
        if chunk is None:
            raise StopAsyncIteration
        return chunk

    # ast-grep-ignore: no-one-line-functions
    async def aclose(self) -> None:
        await self._stream.aclose()


def parameter_text(value: ScalarParam) -> str:
    if value is True:
        return "true"
    if value is False:
        return "false"
    if value is None:
        return ""
    return str(value)


def parameter_entries(name: str, value: ParamValue) -> list[CallValue]:
    match value:
        case list() | tuple():
            return [[name, parameter_text(item)] for item in value]
        case object():
            return [[name, parameter_text(value)]]


def parameter_pairs(params: Params) -> list[CallValue]:
    pairs: list[CallValue] = []
    match params:
        case str() as text:
            for name, value in parse_qsl(text, keep_blank_values=True):
                pairs.append([name, value])
        case bytes() as data:
            for name, value in parse_qsl(data.decode(), keep_blank_values=True):
                pairs.append([name, value])
        case Mapping():
            for name, value in params.items():
                pairs.extend(parameter_entries(name, value))
        case Sequence():
            for name, value in params:
                pairs.extend(parameter_entries(name, value))
    return pairs


def cookie_pairs(cookies: Cookies) -> list[CallValue]:
    match cookies:
        case CookieJar():
            return [[cookie.name, cookie.value] for cookie in cookies]
        case Mapping():
            return [[name, value] for name, value in cookies.items()]
        case Sequence():
            return [[name, value] for name, value in cookies]


def file_fields(files: Files) -> dict[str, CallValue]:
    fields: dict[str, CallValue] = {}
    match files:
        case Mapping():
            for field, content in files.items():
                fields[field] = content
        case Sequence():
            grouped: dict[str, list[CallValue]] = {}
            for field, content in files:
                grouped.setdefault(field, []).append(content)
            for field, items in grouped.items():
                fields[field] = items
    return fields


def content_bytes(content: Content) -> bytes:
    match content:
        case str():
            return content.encode()
        case bytes():
            return content
        case bytearray():
            return bytes(content)
        case memoryview():
            return content.tobytes()
        case Readable():
            return content.read()
        case AsyncIterable():
            raise TypeError("the synchronous route takes content that is iterable, not async")
        case object():
            return b"".join(content)


async def acontent_bytes(content: Content) -> bytes:
    match content:
        case AsyncIterable():
            return b"".join([chunk async for chunk in content])
        case object():
            return content_bytes(content)


def request_values(route: RouteValues) -> dict[str, CallValue]:
    provider_config = route["kwargs"].get("provider_config")
    if provider_config is not None:
        raise UnsupportedCallArgument("provider_config")
    logging_object = route["kwargs"].get("litellm_logging_obj")
    if logging_object is not None:
        raise UnsupportedCallArgument("litellm_logging_obj")
    files = route["files"]
    params = route["params"]
    cookies = route["cookies"]
    return {
        "method": route["method"],
        "endpoint": route["endpoint"],
        "model": route["model"],
        "custom_llm_provider": route["custom_llm_provider"],
        "api_base": route["api_base"],
        "api_key": route["api_key"],
        "request_query_params": route["request_query_params"],
        "request_headers": route["request_headers"],
        "content": route["content"],
        "data": route["data"],
        "files": None if files is None else file_fields(files),
        "json": route["json"],
        "params": None if params is None else parameter_pairs(params),
        "cookies": None if cookies is None else cookie_pairs(cookies),
        "client": route["client"],
        **route["kwargs"],
    }


def reply_response(
    status_code: int, headers: list[tuple[str, str]], body: bytes | NativePassthroughStream
) -> PassthroughResponse | PassthroughStreamingResponse:
    match body:
        case NativePassthroughStream() as stream:
            return PassthroughStreamingResponse(status_code, dict(headers), stream)
        case bytes() as content:
            return PassthroughResponse(status_code, dict(headers), content)


def async_reply_response(
    status_code: int, headers: list[tuple[str, str]], body: bytes | NativePassthroughStream
) -> PassthroughResponse | AsyncPassthroughStreamingResponse:
    match body:
        case NativePassthroughStream() as stream:
            return AsyncPassthroughStreamingResponse(status_code, dict(headers), stream)
        case bytes() as content:
            return PassthroughResponse(status_code, dict(headers), content)


def llm_passthrough_route(
    *,
    method: str,
    endpoint: str,
    model: str,
    custom_llm_provider: str | None = None,
    api_base: str | None = None,
    api_key: str | None = None,
    request_query_params: dict[str, JsonValue] | None = None,
    request_headers: dict[str, str] | None = None,
    content: Content | None = None,
    data: dict[str, CallValue] | None = None,
    files: Files | None = None,
    json: JsonValue = None,
    params: Params | None = None,
    cookies: Cookies | None = None,
    client: SessionLike | None = None,
    **kwargs: CallValue,
) -> PassthroughResponse | PassthroughStreamingResponse:
    values: RouteValues = {
        "method": method,
        "endpoint": endpoint,
        "model": model,
        "custom_llm_provider": custom_llm_provider,
        "api_base": api_base,
        "api_key": api_key,
        "request_query_params": request_query_params,
        "request_headers": request_headers,
        "content": None if content is None else content_bytes(content),
        "data": data,
        "files": files,
        "json": json,
        "params": params,
        "cookies": cookies,
        "client": client,
        "kwargs": kwargs,
    }
    payload = to_call_payload(request_values(values))
    session = NativePassthroughSession("{}")
    try:
        status_code, headers, body = session.call(payload.text, payload.blobs)
    except NativePassthroughError as failure:
        raise_native_error(NativeError(str(failure)))
    return reply_response(status_code, headers, body)


async def allm_passthrough_route(
    *,
    method: str,
    endpoint: str,
    model: str,
    custom_llm_provider: str | None = None,
    api_base: str | None = None,
    api_key: str | None = None,
    request_query_params: dict[str, JsonValue] | None = None,
    request_headers: dict[str, str] | None = None,
    content: Content | None = None,
    data: dict[str, CallValue] | None = None,
    files: Files | None = None,
    json: JsonValue = None,
    params: Params | None = None,
    cookies: Cookies | None = None,
    client: SessionLike | None = None,
    **kwargs: CallValue,
) -> PassthroughResponse | AsyncPassthroughStreamingResponse:
    values: RouteValues = {
        "method": method,
        "endpoint": endpoint,
        "model": model,
        "custom_llm_provider": custom_llm_provider,
        "api_base": api_base,
        "api_key": api_key,
        "request_query_params": request_query_params,
        "request_headers": request_headers,
        "content": None if content is None else await acontent_bytes(content),
        "data": data,
        "files": files,
        "json": json,
        "params": params,
        "cookies": cookies,
        "client": client,
        "kwargs": kwargs,
    }
    payload = to_call_payload(request_values(values))
    session = NativePassthroughSession("{}")
    try:
        status_code, headers, body = await session.acall(payload.text, payload.blobs)
    except NativePassthroughError as failure:
        raise_native_error(NativeError(str(failure)))
    return async_reply_response(status_code, headers, body)
