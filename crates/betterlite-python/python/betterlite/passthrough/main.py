from collections.abc import AsyncIterator, Generator, Iterator, Mapping
from typing import Final, TypeAlias, TypeVar

from pydantic import JsonValue, TypeAdapter

from betterlite._completion.arguments import SessionLike
from betterlite._completion.errors import raise_native_error
from betterlite._endpoints.payload import CallPayload, CallValue, to_call_payload
from betterlite._native import (
    NativeError,
    NativePassthroughError,
    NativePassthroughSession,
    NativePassthroughStream,
)
from betterlite.exceptions import APIError
from betterlite.exceptions._http import Headers, Response
from betterlite.passthrough.utils import (
    Content,
    Cookies,
    Files,
    Params,
    RouteValues,
    acontent_bytes,
    content_bytes,
    request_values,
)

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ERROR_STATUS: Final = 400

ReplyHeaders: TypeAlias = list[tuple[str, str]]
ReplyBody: TypeAlias = bytes | NativePassthroughStream
ResponseParts: TypeAlias = tuple[int, ReplyHeaders, ReplyBody]


def chunk_or_stop(
    chunk: bytes | None, exhausted: type[StopIteration] | type[StopAsyncIteration]
) -> bytes:
    if chunk is None:
        raise exhausted
    return chunk


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
        try:
            chunk = self._stream.next_chunk()
        except NativePassthroughError as failure:
            self.close()
            raise_native_error(NativeError(str(failure)))
        return chunk_or_stop(chunk, StopIteration)

    # ast-grep-ignore: no-one-line-functions
    def close(self) -> None:
        self._stream.close()


StreamReply: TypeAlias = PassthroughResponse | PassthroughStreamingResponse


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

    # ast-grep-ignore: no-one-line-functions
    def aiter_bytes(self) -> "AsyncPassthroughStreamingResponse":
        return self

    async def __anext__(self) -> bytes:
        try:
            chunk = await self._stream.anext_chunk()
        except NativePassthroughError as failure:
            await self.aclose()
            raise_native_error(NativeError(str(failure)))
        return chunk_or_stop(chunk, StopAsyncIteration)

    # ast-grep-ignore: no-one-line-functions
    async def aclose(self) -> None:
        await self._stream.aclose()


# ast-grep-ignore: no-long-names
AsyncStreamReply: TypeAlias = PassthroughResponse | AsyncPassthroughStreamingResponse

# ast-grep-ignore: no-long-names
StreamType = TypeVar("StreamType", PassthroughStreamingResponse, AsyncPassthroughStreamingResponse)


def reply_response(
    status_code: int,
    headers: ReplyHeaders,
    body: ReplyBody,
    streaming: type[StreamType],
) -> PassthroughResponse | StreamType:
    match body:
        case NativePassthroughStream() as stream:
            return streaming(status_code, dict(headers), stream)
        case bytes() as content:
            return PassthroughResponse(status_code, dict(headers), content)


def route_session(route: RouteValues) -> tuple[CallPayload, NativePassthroughSession]:
    payload = to_call_payload(request_values(route))
    return payload, NativePassthroughSession("{}")


def call_route(route: RouteValues) -> ResponseParts:
    payload, session = route_session(route)
    try:
        parts = session.call(payload.text, payload.blobs)
    except NativePassthroughError as failure:
        raise_native_error(NativeError(str(failure)))
    return parts


async def acall_route(route: RouteValues) -> ResponseParts:
    payload, session = route_session(route)
    try:
        parts = await session.acall(payload.text, payload.blobs)
    except NativePassthroughError as failure:
        raise_native_error(NativeError(str(failure)))
    return parts


def llm_passthrough_route(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/passthrough/main.py#L393
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
) -> StreamReply:
    route = RouteValues(
        method=method,
        endpoint=endpoint,
        model=model,
        custom_llm_provider=custom_llm_provider,
        api_base=api_base,
        api_key=api_key,
        request_query_params=request_query_params,
        request_headers=request_headers,
        content=None if content is None else content_bytes(content),
        data=data,
        files=files,
        json=json,
        params=params,
        cookies=cookies,
        client=client,
        kwargs=kwargs,
    )
    return reply_response(*call_route(route), PassthroughStreamingResponse)


async def allm_passthrough_route(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/passthrough/main.py#L268
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
) -> AsyncStreamReply:
    body = None if content is None else await acontent_bytes(content)
    route = RouteValues(
        method=method,
        endpoint=endpoint,
        model=model,
        custom_llm_provider=custom_llm_provider,
        api_base=api_base,
        api_key=api_key,
        request_query_params=request_query_params,
        request_headers=request_headers,
        content=body,
        data=data,
        files=files,
        json=json,
        params=params,
        cookies=cookies,
        client=client,
        kwargs=kwargs,
    )
    # ast-grep-ignore: no-long-names
    return reply_response(*await acall_route(route), AsyncPassthroughStreamingResponse)
