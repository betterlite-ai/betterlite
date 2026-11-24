import asyncio
import atexit
import datetime as dt
import os
import sys
from collections.abc import AsyncIterator, Coroutine
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, Unpack

import httpx
import litellm
import openai
from litellm.exceptions import BadRequestError
from litellm.files.types import FileContentStreamingResult
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.llms.openai import HttpxBinaryResponseContent
from openai import OpenAI
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

from fine_tuning_read import Failure, StructuredTimeout, api_failure, timeout_value

# a supplied HTTPHandler's httpx client adds this header, so the wire shows when it sent the call
HANDLER_HEADERS: dict[str, str] = {"x-supplied-client": "http-handler"}
DEFAULT_CHUNK_SIZE: Final = 1024 * 1024

# LiteLLM's own 400 class, told apart from the SDK's status errors by an isinstance against this name
BAD_REQUEST: Final[type[Exception]] = BadRequestError

Function: TypeAlias = Literal["file_content", "afile_content"]
Supplied: TypeAlias = Literal["none", "openai_sdk", "azure_sdk", "http_handler"]

# the buffered call answers a binary response and the streaming one a byte stream
FileValue: TypeAlias = HttpxBinaryResponseContent | FileContentStreamingResult
FileResult: TypeAlias = FileValue | Coroutine[object, object, FileValue]


class FileValues(TypedDict, total=False):
    file_id: str
    model: str | None
    custom_llm_provider: str | None
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None
    chunk_size: int
    stream: bool
    api_base: str
    api_key: str | None
    api_version: str | None
    organization: str | None
    timeout: float | str | httpx.Timeout | None
    request_timeout: float | str | None
    max_retries: int | None
    client: OpenAI | HTTPHandler
    litellm_logging_obj: Logging


class FileContentCall(Protocol):
    def __call__(self, **values: Unpack[FileValues]) -> FileResult: ...


# litellm leaves both names unannotated, so every name on the module answers one typed call
class FileContentApi(Protocol):
    def __getattr__(self, name: str) -> FileContentCall: ...


API: FileContentApi = litellm


# LiteLLM decides the exit status itself, so a finished case flushes and ends the process
def end_after_output() -> None:
    sys.stdout.flush()
    os._exit(0)


class Step(BaseModel):
    model_config = ConfigDict(extra="forbid")

    function: Function
    file_id: str
    model: str | None = None
    custom_llm_provider: str | None = None
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, str] | None = None
    api_key: str | None = None
    api_version: str | None = None
    organization: str | None = None
    timeout: float | str | StructuredTimeout | None = None
    request_timeout: float | str | None = None
    max_retries: int | None = None
    chunk_size: int | None = None
    stream: bool = False
    client: Supplied = "none"
    supplied_logging: bool = False


class FileContentStreamCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    steps: list[Step]


class Placeholder(BaseModel):
    model_config = ConfigDict(extra="forbid")

    body: str
    method: str
    url: str
    model: str
    provider: str


class Buffered(BaseModel):
    model_config = ConfigDict(extra="forbid")

    status: int
    headers: dict[str, str]
    body: list[int]


class Streamed(BaseModel):
    model_config = ConfigDict(extra="forbid")

    chunks: list[list[int]]
    headers: dict[str, str]


class Identity(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    details_model: str
    custom_llm_provider: str
    api_base: str | None = None


class StepOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    buffered: Buffered | None = None
    streamed: Streamed | None = None
    identity: Identity | None = None
    failure: Failure | None = None
    placeholder: Placeholder | None = None


class Result(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_content_stream: dict[str, dict[str, list[StepOutcome]]]


class FileContentStreamRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_content_stream: FileContentStreamCall

    def serialize(self, api_base: str) -> str:
        steps = [run_step(step, api_base) for step in self.file_content_stream.steps]
        result = Result(file_content_stream={"value": {"steps": steps}})
        atexit.register(end_after_output)
        return result.model_dump_json(by_alias=True, exclude_unset=True)


# LiteLLM's model_call_details as the success callback receives it
class Details(TypedDict, total=False):
    model: str
    custom_llm_provider: str
    litellm_params: dict[str, JsonValue]


class Callback(TypedDict, total=False):
    kwargs: Details


DETAILS: TypeAdapter[Callback] = TypeAdapter(Callback)
API_BASE: TypeAdapter[str | None] = TypeAdapter(str | None)


class Capture(CustomLogger):
    success: list[dict[str, JsonValue]]

    def __init__(self) -> None:
        self.success = []

    @override
    async def async_log_success_event(self, *args: JsonValue, **kwargs: JsonValue) -> None:
        del args
        self.success.append(kwargs)


def logging_fixture(capture: Capture) -> Logging:
    logging_obj = Logging(
        model="",
        messages=[],
        stream=False,
        call_type="afile_content",
        start_time=dt.datetime(2026, 1, 1, tzinfo=dt.UTC),
        litellm_call_id="file-content-stream",
        function_id="file-content-stream",
    )
    logging_obj.add_dynamic_callback(capture)
    return logging_obj


# the success callback is the only place the written identity surfaces, and only file_content_streaming writes the provider
def identity(logging_obj: Logging, capture: Capture) -> Identity | None:
    if not capture.success:
        return None
    callback = DETAILS.validate_python(capture.success[-1])
    details = callback.get("kwargs")
    if details is None:
        return None
    provider = details.get("custom_llm_provider")
    if provider is None:
        return None
    params = details.get("litellm_params") or {}
    return Identity(
        model=logging_obj.model,
        details_model=details.get("model") or "",
        custom_llm_provider=provider,
        api_base=API_BASE.validate_python(params.get("api_base")),
    )


def buffered(value: HttpxBinaryResponseContent) -> Buffered:
    response = value.response
    return Buffered(
        status=response.status_code,
        headers=dict(response.headers),
        body=list(response.content),
    )


def streamed_sync(result: FileContentStreamingResult) -> Streamed:
    iterator = result.stream_iterator
    if isinstance(iterator, AsyncIterator):
        raise TypeError("the LiteLLM file stream is not sync")
    return Streamed(
        chunks=[list(chunk) for chunk in iterator],
        headers=dict(result.headers),
    )


async def streamed_async(result: FileContentStreamingResult) -> Streamed:
    iterator = result.stream_iterator
    if not isinstance(iterator, AsyncIterator):
        raise TypeError("the LiteLLM file stream is not async")
    chunks = [list(chunk) async for chunk in iterator]
    return Streamed(chunks=chunks, headers=dict(result.headers))


async def settled_async(awaited: Coroutine[object, object, FileValue]) -> Buffered | Streamed:
    result = await awaited
    if isinstance(result, FileContentStreamingResult):
        return await streamed_async(result)
    if isinstance(result, HttpxBinaryResponseContent):
        return buffered(result)
    raise TypeError(f"the SDK returned an unknown file content answer: {type(result).__name__}")


def settle(result: FileResult) -> Buffered | Streamed:
    if isinstance(result, Coroutine):
        with asyncio.Runner() as runner:
            return runner.run(settled_async(result))
    if isinstance(result, FileContentStreamingResult):
        return streamed_sync(result)
    if isinstance(result, HttpxBinaryResponseContent):
        return buffered(result)
    raise TypeError(f"the SDK returned an unknown file content answer: {type(result).__name__}")


def placeholder(error: openai.APIStatusError, provider: str) -> Placeholder:
    request = error.response.request
    return Placeholder(
        body=error.response.text,
        method=request.method,
        url=str(request.url),
        model="n/a",
        provider=provider,
    )


def content_values(step: Step, api_base: str, logging_obj: Logging | None) -> FileValues:
    values = FileValues(
        file_id=step.file_id,
        extra_headers=step.extra_headers,
        extra_body=step.extra_body,
        chunk_size=step.chunk_size if step.chunk_size is not None else DEFAULT_CHUNK_SIZE,
        stream=step.stream,
        api_base=api_base,
        api_key=step.api_key,
        api_version=step.api_version,
        organization=step.organization,
        timeout=timeout_value(step.timeout),
        request_timeout=step.request_timeout,
        max_retries=step.max_retries,
    )
    # afile_content's declared default is "openai", so an unnamed provider must stay absent rather than null
    if step.model is not None:
        values["model"] = step.model
    if step.custom_llm_provider is not None:
        values["custom_llm_provider"] = step.custom_llm_provider
    match step.client:
        case "openai_sdk":
            values["client"] = OpenAI(base_url=f"{api_base}/supplied", api_key="sk-supplied")
        case "http_handler":
            values["client"] = HTTPHandler(client=httpx.Client(headers=HANDLER_HEADERS))
        case "azure_sdk" | "none":
            pass
    if logging_obj is not None:
        values["litellm_logging_obj"] = logging_obj
    return values


def call_litellm(function: Function, values: FileValues) -> FileResult:
    match function:
        case "file_content":
            return API.file_content(**values)
        case "afile_content":
            return API.afile_content(**values)


def run_step(step: Step, api_base: str) -> StepOutcome:
    capture = Capture()
    logging_obj = logging_fixture(capture) if step.supplied_logging else None
    values = content_values(step, api_base, logging_obj)
    try:
        settled = settle(call_litellm(step.function, values))
    except openai.APIStatusError as error:
        # LiteLLM's rejected-provider 400 carries the placeholder response; a provider status error does not
        if isinstance(error, BAD_REQUEST):
            provider = step.custom_llm_provider or "openai"
            return StepOutcome(
                failure=api_failure(error), placeholder=placeholder(error, provider)
            )
        return StepOutcome(failure=api_failure(error))
    except ValueError as error:
        return StepOutcome(failure=Failure(cls=type(error).__name__, status=0, message=str(error)))
    told = identity(logging_obj, capture) if logging_obj is not None and step.stream else None
    outcome = (
        StepOutcome(buffered=settled)
        if isinstance(settled, Buffered)
        else StepOutcome(streamed=settled)
    )
    # a field is written only when it has a value, so an absent identity stays out of the JSON
    if told is not None:
        outcome.identity = told
    return outcome
