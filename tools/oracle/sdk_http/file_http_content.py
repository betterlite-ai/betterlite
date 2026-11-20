import asyncio
from collections.abc import AsyncIterator, Coroutine, Iterator
from datetime import UTC, datetime
from typing import Final, Literal

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.files.transformation import BaseFilesConfig
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.llms.openai import FileContentRequest, HttpxBinaryResponseContent
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, ConfigDict, Field, JsonValue

Mode = Literal["sync", "async", "streaming"]

DEFAULT_CHUNK_SIZE: Final = 1024 * 1024
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException
HANDLER: Final = BaseLLMHTTPHandler()


class FileHttpContentCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    mode: Mode
    file_id: str
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, str] | None = None
    chunk_size: int | None = None
    timeout_seconds: int | None = None


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int = Field(serialization_alias="status")
    status_code_is_synthesized: bool = Field(serialization_alias="synthesized")
    message: str
    headers: dict[str, str]


class BufferedContent(BaseModel):
    status: int
    headers: dict[str, str]
    body: list[int]


class StreamedContent(BaseModel):
    chunks: list[list[int]]
    headers: dict[str, str]


class Outcome(BaseModel):
    result: BufferedContent | StreamedContent | None = None
    failure: Failure | None = None


class ContentValue(BaseModel):
    value: Outcome


class ContentEnvelope(BaseModel):
    file_http_content: ContentValue


def provider_config() -> BaseFilesConfig:
    config = ProviderConfigManager.get_provider_files_config("", LlmProviders.MANUS)
    if config is None:
        raise TypeError("LiteLLM has no files config for manus")
    return config


def logging_fixture(call_type: str) -> Logging:
    return Logging(
        model="",
        messages="",
        stream=False,
        call_type=call_type,
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="file-http-content",
        function_id="file-http-content",
    )


def request_body(call: FileHttpContentCall) -> FileContentRequest:
    return FileContentRequest(
        file_id=call.file_id,
        extra_headers=call.extra_headers,
        extra_body=call.extra_body,
    )


# file_content hands the handler the caller's extra headers as its headers argument
def caller_headers(call: FileHttpContentCall) -> dict[str, str]:
    if call.extra_headers is None:
        return {}
    return dict(call.extra_headers)


def content_timeout(call: FileHttpContentCall) -> float | None:
    if call.timeout_seconds is None:
        return None
    return float(call.timeout_seconds)


def buffered_outcome(content: HttpxBinaryResponseContent) -> Outcome:
    response = content.response
    result = BufferedContent(
        status=response.status_code,
        headers=dict(response.headers),
        body=list(response.content),
    )
    return Outcome(result=result)


async def drain_chunks(stream: Iterator[bytes] | AsyncIterator[bytes]) -> list[list[int]]:
    match stream:
        case AsyncIterator():
            return [list(chunk) async for chunk in stream]
        case Iterator():
            return [list(chunk) for chunk in stream]


def sync_content(call: FileHttpContentCall, api_base: str) -> HttpxBinaryResponseContent:
    litellm_params: dict[str, JsonValue] = {"api_base": api_base}
    result = HANDLER.retrieve_file_content(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L5007-L5017
        file_content_request=request_body(call),
        provider_config=provider_config(),
        litellm_params=litellm_params,
        headers=caller_headers(call),
        logging_obj=logging_fixture("file_content"),
        timeout=content_timeout(call),
    )
    match result:
        case HttpxBinaryResponseContent():
            return result
        case Coroutine():
            result.close()
            raise TypeError("the sync content handler returned a coroutine")


async def async_content(call: FileHttpContentCall, api_base: str) -> HttpxBinaryResponseContent:
    litellm_params: dict[str, JsonValue] = {"api_base": api_base}
    return await HANDLER.async_retrieve_file_content(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L5063-L5072
        file_content_request=request_body(call),
        provider_config=provider_config(),
        litellm_params=litellm_params,
        headers=caller_headers(call),
        logging_obj=logging_fixture("afile_content"),
        timeout=content_timeout(call),
    )


async def streaming_view(call: FileHttpContentCall, api_base: str) -> Outcome:
    litellm_params: dict[str, JsonValue] = {"api_base": api_base}
    result = await HANDLER.async_retrieve_file_content_streaming(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L5109-L5119
        file_content_request=request_body(call),
        provider_config=provider_config(),
        litellm_params=litellm_params,
        headers=caller_headers(call),
        logging_obj=logging_fixture("afile_content"),
        chunk_size=DEFAULT_CHUNK_SIZE if call.chunk_size is None else call.chunk_size,
        timeout=content_timeout(call),
    )
    chunks = await drain_chunks(result.stream_iterator)
    return Outcome(result=StreamedContent(chunks=chunks, headers=dict(result.headers)))


def run_content(call: FileHttpContentCall, api_base: str) -> Outcome:
    value: Outcome
    try:
        match call.mode:
            case "sync":
                value = buffered_outcome(sync_content(call, api_base))
            case "async":
                value = buffered_outcome(asyncio.run(async_content(call, api_base)))
            case "streaming":
                value = asyncio.run(streaming_view(call, api_base))
    except PROVIDER_ERROR as error:
        return Outcome(failure=Failure.model_validate(error))
    return value


class FileHttpContentRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_http_content: FileHttpContentCall

    def serialize(self, api_base: str) -> str:
        outcome = run_content(self.file_http_content, api_base)
        value = ContentValue(value=outcome)
        return ContentEnvelope(file_http_content=value).model_dump_json(by_alias=True)
