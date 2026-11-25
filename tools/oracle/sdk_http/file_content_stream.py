import asyncio
import atexit
import datetime as dt
import os
import sys
from collections.abc import AsyncIterator, Coroutine, Iterator
from typing import Literal, Protocol, Required, TypeAlias, TypedDict, Unpack

import httpx
import litellm
import openai
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

Function: TypeAlias = Literal["file_content", "afile_content"]
FileValue: TypeAlias = HttpxBinaryResponseContent | FileContentStreamingResult
FileResult: TypeAlias = FileValue | Coroutine[None, None, FileValue]


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
    azure_ad_token: str | None
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
    azure_ad_token: str | None = None
    api_version: str | None = None
    organization: str | None = None
    timeout: float | str | StructuredTimeout | None = None
    request_timeout: float | str | None = None
    max_retries: int | None = None
    chunk_size: int | None = None
    stream: bool = False
    client: Literal["none", "openai_sdk", "azure_sdk", "http_handler"] = "none"
    supplied_logging: bool = False


class Placeholder(BaseModel):
    body: str
    method: str
    url: str
    model: str
    provider: str


class Buffered(BaseModel):
    status: int
    headers: dict[str, str]
    body: list[int]


class Streamed(BaseModel):
    chunks: list[list[int]]
    headers: dict[str, str]


class Identity(BaseModel):
    model: str
    details_model: str
    custom_llm_provider: str
    api_base: str | None = None


class StepOutcome(BaseModel):
    buffered: Buffered | None = None
    streamed: Streamed | None = None
    identity: Identity | None = None
    failure: Failure | None = None
    placeholder: Placeholder | None = None


class Result(BaseModel):
    file_content_stream: dict[str, dict[str, list[StepOutcome]]]


class FileContentStreamRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_content_stream: dict[str, list[Step]]

    def serialize(self, api_base: str) -> str:
        steps = [run_step(step, api_base) for step in self.file_content_stream["steps"]]
        result = Result(file_content_stream={"value": {"steps": steps}})
        atexit.register(end_after_output)
        return result.model_dump_json(by_alias=True, exclude_unset=True)


# LiteLLM's model_call_details as the success callback receives it
class Details(TypedDict, total=False):
    model: Required[str]
    custom_llm_provider: str
    litellm_params: dict[str, JsonValue]


DETAILS: TypeAdapter[Details] = TypeAdapter(Details)
API_BASE: TypeAdapter[str | None] = TypeAdapter(str | None)


class Capture(CustomLogger):
    success: list[dict[str, JsonValue]]

    def __init__(self) -> None:
        self.success = []

    @override
    async def async_log_success_event(self, *args: JsonValue, **kwargs: JsonValue) -> None:
        del args
        self.success.append(kwargs)


# only file_content_streaming writes the provider, which its success callback reports
def identity(logging_obj: Logging, capture: Capture) -> Identity | None:
    if not capture.success:
        return None
    details = DETAILS.validate_python(capture.success[-1]["kwargs"])
    provider = details.get("custom_llm_provider")
    if provider is None:
        return None
    params = details.get("litellm_params")
    api_base = None if params is None else API_BASE.validate_python(params.get("api_base"))
    return Identity(model=logging_obj.model, details_model=details["model"],
                    custom_llm_provider=provider, api_base=api_base)


def buffered(value: HttpxBinaryResponseContent) -> Buffered:
    response = value.response
    return Buffered(
        status=response.status_code, headers=dict(response.headers), body=list(response.content)
    )


def streamed_sync(result: FileContentStreamingResult) -> Streamed:
    # LiteLLM's wrapper implements both protocols; the SDK call selects which one to use.
    match result.stream_iterator:
        case Iterator() as iterator:
            return Streamed(
                chunks=[list(chunk) for chunk in iterator], headers=dict(result.headers)
            )
        case AsyncIterator():
            raise TypeError("the LiteLLM file stream is not sync")


async def streamed_async(result: FileContentStreamingResult) -> Streamed:
    match result.stream_iterator:
        case AsyncIterator() as iterator:
            chunks = [list(chunk) async for chunk in iterator]
            return Streamed(chunks=chunks, headers=dict(result.headers))
        case Iterator():
            raise TypeError("the LiteLLM file stream is not async")


async def settled_async(awaited: Coroutine[None, None, FileValue]) -> Buffered | Streamed:
    result = await awaited
    match result:
        case FileContentStreamingResult():
            return await streamed_async(result)
        case HttpxBinaryResponseContent():
            return buffered(result)


def settle(result: FileResult) -> Buffered | Streamed:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                return runner.run(settled_async(result))
        case FileContentStreamingResult():
            return streamed_sync(result)
        case HttpxBinaryResponseContent():
            return buffered(result)


def placeholder(error: openai.APIStatusError, provider: str) -> Placeholder:
    request = error.response.request
    return Placeholder(body=error.response.text, method=request.method,
                       url=str(request.url), model="n/a", provider=provider)


def content_values(step: Step, api_base: str, logging_obj: Logging | None) -> FileValues:
    values = FileValues(
        file_id=step.file_id, extra_headers=step.extra_headers,
        extra_body=step.extra_body, stream=step.stream,
        chunk_size=step.chunk_size if step.chunk_size is not None else 1024 * 1024,
        api_base=api_base, api_key=step.api_key,
        azure_ad_token=step.azure_ad_token,
        api_version=step.api_version, organization=step.organization,
        timeout=timeout_value(step.timeout), request_timeout=step.request_timeout,
        max_retries=step.max_retries,
    )
    # afile_content defaults the provider to openai, so an unnamed one stays absent rather than null
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
    logging_obj: Logging | None = None
    if step.supplied_logging:
        logging_obj = Logging(
            model="", messages=None, stream=False, call_type="afile_content",
            start_time=dt.datetime(2026, 1, 1, tzinfo=dt.UTC),
            litellm_call_id="file-content-stream", function_id="file-content-stream",
        )
        logging_obj.add_dynamic_callback(capture)
    values = content_values(step, api_base, logging_obj)
    try:
        settled = settle(call_litellm(step.function, values))
    except openai.BadRequestError as error:
        named = step.custom_llm_provider
        if named is None:
            raise ValueError("a rejected-provider 400 names its provider") from error
        return StepOutcome(failure=api_failure(error), placeholder=placeholder(error, named))
    except openai.APIStatusError as error:
        return StepOutcome(failure=api_failure(error))
    except ValueError as error:
        return StepOutcome(failure=Failure(cls=type(error).__name__, status=0, message=str(error)))
    outcome = StepOutcome()
    match settled:
        case Buffered():
            outcome.buffered = settled
        case Streamed():
            outcome.streamed = settled
    told = identity(logging_obj, capture) if logging_obj is not None and step.stream else None
    if told is not None:
        outcome.identity = told
    return outcome
