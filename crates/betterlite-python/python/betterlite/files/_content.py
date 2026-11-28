import json
from collections.abc import Awaitable, Mapping
from typing import Final, NamedTuple, TypeAlias

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite._completion.arguments import timeout_value
from betterlite._completion.errors import UnsupportedCallArgument, raise_native_error
from betterlite._endpoints.payload import CallPayload, CallValue, to_call_payload
from betterlite._endpoints.proxy import proxy_headers
from betterlite._endpoints.settings import read_settings
from betterlite._native import NativeFileContentError, NativeFileContentSession, NativeFileStream
from betterlite.exceptions._http import Response
from betterlite.files.streaming import FileContentStreamingResponse
from betterlite.files.types import FileContentProvider, FileContentStreamingResult

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
MODEL: Final[TypeAdapter[str | None]] = TypeAdapter(str | None)
HEADERS: Final[TypeAdapter[dict[str, str]]] = TypeAdapter(dict[str, str])

# the betterlite module globals the Rust session resolves its section settings from
SETTING_NAMES: Final[tuple[str, ...]] = (
    "api_base", "api_key", "api_version", "azure_key", "openai_key", "organization",
    "vertex_location", "vertex_project",
)


class FileAnswerText(TypedDict):
    headers: list[tuple[str, str]]


class PlainAnswerText(FileAnswerText):
    status: int


class StreamAnswerText(FileAnswerText):
    custom_llm_provider: str | None


PLAIN: Final[TypeAdapter[PlainAnswerText]] = TypeAdapter(PlainAnswerText)
STREAM: Final[TypeAdapter[StreamAnswerText]] = TypeAdapter(StreamAnswerText)


class FileCallArguments(NamedTuple):
    file_id: str
    model: str | None
    custom_llm_provider: str | None
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None
    chunk_size: int
    stream: bool


class HttpxBinaryResponseContent(Response):
    _hidden_params: dict[str, JsonValue]

    def __init__(self, status_code: int, headers: Mapping[str, str], content: bytes) -> None:
        super().__init__(status_code=status_code, headers=headers, content=content)
        self._hidden_params = {}

    def json(self) -> JsonValue:
        return JSON.validate_json(self.content)

    def read(self) -> bytes:
        return self.content


FileContentOutcome: TypeAlias = HttpxBinaryResponseContent | FileContentStreamingResult
FileAnswer: TypeAlias = tuple[str, list[bytes], NativeFileStream | None]


def file_content_session() -> NativeFileContentSession:
    settings: dict[str, JsonValue] = {"settings": read_settings(SETTING_NAMES)}
    return NativeFileContentSession(json.dumps(settings))


def timeout_payload(value: CallValue | None) -> JsonValue:
    if value is None:
        return None
    return timeout_value(value)


def merged_headers(extra_headers: dict[str, str] | None) -> dict[str, str]:
    merged: dict[str, str] = {}
    if extra_headers is not None:
        merged |= HEADERS.validate_python(extra_headers)
    merged |= proxy_headers()
    return merged


def call_payload_values(
    arguments: FileCallArguments, options: Mapping[str, CallValue]
) -> dict[str, CallValue]:
    if options.get("azure_ad_token_provider") is not None:
        raise UnsupportedCallArgument("azure_ad_token_provider")
    return {
        "file_id": arguments.file_id,
        "model": arguments.model,
        "custom_llm_provider": arguments.custom_llm_provider,
        "extra_headers": merged_headers(arguments.extra_headers),
        "extra_body": arguments.extra_body,
        "chunk_size": arguments.chunk_size,
        "stream": arguments.stream,
        "api_key": options.get("api_key"),
        "azure_ad_token": options.get("azure_ad_token"),
        "tenant_id": options.get("tenant_id"),
        "client_id": options.get("client_id"),
        "client_secret": options.get("client_secret"),
        "azure_username": options.get("azure_username"),
        "azure_password": options.get("azure_password"),
        "azure_scope": options.get("azure_scope"),

        "api_base": options.get("api_base"),
        "api_version": options.get("api_version"),
        "organization": options.get("organization"),
        "timeout": timeout_payload(options.get("timeout")),
        "request_timeout": timeout_payload(options.get("request_timeout")),
        "max_retries": options.get("max_retries"),
        "litellm_call_id": options.get("litellm_call_id"),
        "id": options.get("id"),
        "client": options.get("client"),
        "shared_session": options.get("shared_session"),
        "logger_fn": options.get("logger_fn"),
    }


def native_answer(session: NativeFileContentSession, payload: CallPayload) -> FileAnswer:
    try:
        return session.content(payload.text, payload.blobs)
    except NativeFileContentError as failure:
        raise_native_error(failure)


async def async_native_answer(
    session: NativeFileContentSession, payload: CallPayload
) -> FileAnswer:
    try:
        return await session.acontent(payload.text, payload.blobs)
    except NativeFileContentError as failure:
        raise_native_error(failure)


def answer_result(
    text: str, blobs: list[bytes], stream: NativeFileStream | None, call: FileCallArguments
) -> FileContentOutcome:
    if stream is None:
        plain = PLAIN.validate_json(text)
        return HttpxBinaryResponseContent(plain["status"], dict(plain["headers"]), blobs[0])
    streamed = STREAM.validate_json(text)
    wrapper = FileContentStreamingResponse(
        stream_iterator=stream,
        file_id=call.file_id,
        model=call.model,
        custom_llm_provider=streamed["custom_llm_provider"],
    )
    return FileContentStreamingResult(stream_iterator=wrapper, headers=dict(streamed["headers"]))


def sync_content(
    arguments: FileCallArguments, options: Mapping[str, CallValue]
) -> FileContentOutcome:
    payload = to_call_payload(call_payload_values(arguments, options))
    session = file_content_session()
    text, blobs, stream = native_answer(session, payload)
    return answer_result(text, blobs, stream, arguments)


async def async_content(
    arguments: FileCallArguments, options: Mapping[str, CallValue]
) -> FileContentOutcome:
    payload = to_call_payload(call_payload_values(arguments, options))
    session = file_content_session()
    text, blobs, stream = await async_native_answer(session, payload)
    return answer_result(text, blobs, stream, arguments)


def file_content(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/files/main.py#L853-L860
    file_id: str,
    model: str | None = None,
    custom_llm_provider: FileContentProvider | str | None = None,
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, str] | None = None,
    chunk_size: int = 1024 * 1024,
    stream: bool = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/files/main.py#L860
    **kwargs: CallValue,
) -> FileContentOutcome | Awaitable[FileContentOutcome]:
    arguments = FileCallArguments(
        file_id=file_id,
        model=model,
        custom_llm_provider=custom_llm_provider,
        extra_headers=extra_headers,
        extra_body=extra_body,
        chunk_size=chunk_size,
        stream=stream,
    )
    if kwargs.get("afile_content") is True:
        return async_content(arguments, kwargs)
    return sync_content(arguments, kwargs)


async def afile_content(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/files/main.py#L806-L812
    file_id: str,
    custom_llm_provider: FileContentProvider = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, str] | None = None,
    chunk_size: int = 1024 * 1024,
    stream: bool = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/files/main.py#L812
    **kwargs: CallValue,
) -> FileContentOutcome:
    arguments = FileCallArguments(
        file_id=file_id,
        model=MODEL.validate_python(kwargs.get("model")),
        custom_llm_provider=custom_llm_provider,
        extra_headers=extra_headers,
        extra_body=extra_body,
        chunk_size=chunk_size,
        stream=stream,
    )
    return await async_content(arguments, kwargs)
