import asyncio
import atexit
import os
import sys
from collections.abc import Coroutine
from typing import Literal, Protocol, TypeAlias, TypedDict, Unpack, cast, runtime_checkable

import httpx
import litellm
import openai
from litellm.exceptions import BadRequestError
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.llms.openai import OpenAIFileObject
from openai import OpenAI
from pydantic import BaseModel, ConfigDict, JsonValue

from fine_tuning_read import JSON, Failure, StructuredTimeout, api_failure, timeout_value

# a supplied HTTPHandler's httpx client adds this header, so the wire shows when it sent the call
HANDLER_HEADERS: dict[str, str] = {"x-supplied-client": "http-handler"}

Function: TypeAlias = Literal["file_retrieve", "afile_retrieve", "file_list", "afile_list"]
Supplied: TypeAlias = Literal["none", "openai_sdk", "http_handler"]

# the OpenAI and azure branches answer the SDK's object or page, a provider config a file list
Answer: TypeAlias = BaseModel | list[OpenAIFileObject]
ReadResult: TypeAlias = Answer | Coroutine[object, object, Answer]


class ReadValues(TypedDict, total=False):
    file_id: str
    purpose: str | None
    custom_llm_provider: str
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None
    api_base: str
    api_key: str | None
    api_version: str | None
    organization: str | None
    timeout: float | str | httpx.Timeout | None
    request_timeout: float | str | None
    max_retries: int | None
    is_async: bool
    after: str | None
    limit: int | None
    order: str | None
    client: OpenAI | HTTPHandler


class ReadCall(Protocol):
    def __call__(self, **values: Unpack[ReadValues]) -> ReadResult: ...


# @client hides the four signatures, so every name on the module answers one typed call
class FileReadApi(Protocol):
    def __getattr__(self, name: str) -> ReadCall: ...


API: FileReadApi = litellm


# the harness reads the exit status rather than the output, so the adapter ends the process once the result is written
def end_after_output() -> None:
    sys.stdout.flush()
    os._exit(0)


atexit.register(end_after_output)


# BadRequestError leaves model and llm_provider unannotated; both are the text LiteLLM passes
class ProviderFailure(Protocol):
    model: str
    llm_provider: str


class Step(BaseModel):
    model_config = ConfigDict(extra="forbid")

    function: Function
    file_id: str = ""
    purpose: str | None = None
    custom_llm_provider: str = "openai"
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, str] | None = None
    api_key: str | None = None
    api_version: str | None = None
    organization: str | None = None
    timeout: float | str | StructuredTimeout | None = None
    request_timeout: float | str | None = None
    max_retries: int | None = None
    is_async: bool = False
    after: str | None = None
    limit: int | None = None
    order: str | None = None
    client: Supplied = "none"


class FileProviderReadCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    steps: list[Step]


class Placeholder(BaseModel):
    model_config = ConfigDict(extra="forbid")

    body: str
    method: str
    url: str
    model: str
    provider: str


class StepOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file: JsonValue = None
    listing: JsonValue = None
    failure: Failure | None = None
    placeholder: Placeholder | None = None


class Result(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_provider_read: dict[str, dict[str, list[StepOutcome]]]


class FileProviderReadRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_provider_read: FileProviderReadCall

    def serialize(self, api_base: str) -> str:
        steps: list[StepOutcome] = []
        raised = False
        for step in self.file_provider_read.steps:
            try:
                steps.append(run_step(step, api_base))
            except BaseException as error:
                steps.append(raised_step(step, error))
                raised = True
                break
        if not raised:
            steps.append(StepOutcome(file={"!no-step-raised": True}))
        result = Result(file_provider_read={"value": {"steps": steps}})
        return result.model_dump_json(by_alias=True, exclude_unset=True)


def read_values(step: Step, api_base: str) -> ReadValues:
    values = ReadValues(
        custom_llm_provider=step.custom_llm_provider,
        extra_headers=step.extra_headers,
        extra_body=step.extra_body,
        api_base=api_base,
        api_key=step.api_key,
        api_version=step.api_version,
        organization=step.organization,
        timeout=timeout_value(step.timeout),
        request_timeout=step.request_timeout,
        max_retries=step.max_retries,
        is_async=step.is_async,
        after=step.after,
        limit=step.limit,
        order=step.order,
    )
    match step.function:
        case "file_retrieve" | "afile_retrieve":
            values["file_id"] = step.file_id
        case "file_list" | "afile_list":
            values["purpose"] = step.purpose
    match step.client:
        case "openai_sdk":
            values["client"] = OpenAI(base_url=f"{api_base}/supplied", api_key="sk-supplied")
        case "http_handler":
            values["client"] = HTTPHandler(client=httpx.Client(headers=HANDLER_HEADERS))
        case "none":
            pass
    return values


def call_litellm(function: Function, values: ReadValues) -> ReadResult:
    match function:
        case "file_retrieve":
            return API.file_retrieve(**values)
        case "afile_retrieve":
            return API.afile_retrieve(**values)
        case "file_list":
            return API.file_list(**values)
        case "afile_list":
            return API.afile_list(**values)


@runtime_checkable
class HasDump(Protocol):
    def model_dump_json(self, *, exclude_none: bool = False) -> str: ...


def settle(result: ReadResult) -> JsonValue:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                return settle(runner.run(result))
        case list():
            return [JSON.validate_json(file.model_dump_json(exclude_none=True)) for file in result]
        case HasDump():
            return JSON.validate_json(result.model_dump_json(exclude_none=True))


def placeholder(error: BadRequestError) -> Placeholder:
    raised = cast(ProviderFailure, error)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L219
    request = error.response.request
    return Placeholder(
        body=error.response.text,
        method=request.method,
        url=str(request.url),
        model=raised.model,
        provider=raised.llm_provider,
    )


def run_step(step: Step, api_base: str) -> StepOutcome:
    try:
        settled = settle(call_litellm(step.function, read_values(step, api_base)))
    except BadRequestError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L219
        return StepOutcome(failure=api_failure(error), placeholder=placeholder(error))
    except openai.APIStatusError as error:
        return StepOutcome(failure=api_failure(error))
    except ValueError as error:
        return StepOutcome(failure=Failure(cls=type(error).__name__, status=0, message=str(error)))
    match step.function:
        case "file_retrieve" | "afile_retrieve":
            return StepOutcome(file=settled)
        case "file_list" | "afile_list":
            return StepOutcome(listing=settled)


# a step that raised keeps its exception text in the compared value, where the oracle's pointer shows it
def raised_step(step: Step, error: BaseException) -> StepOutcome:
    trace = error.__traceback__
    while trace is not None and trace.tb_next is not None:
        trace = trace.tb_next
    where = ""
    if trace is not None:
        source = trace.tb_frame.f_code.co_filename.rsplit("/", maxsplit=1)[-1]
        where = f" at {source}:{trace.tb_lineno}"
    label = " ".join(f"!{type(error).__name__}: {error}{where}".split())[:300]
    match step.function:
        case "file_retrieve" | "afile_retrieve":
            return StepOutcome(file={label: True})
        case "file_list" | "afile_list":
            return StepOutcome(listing={label: True})
