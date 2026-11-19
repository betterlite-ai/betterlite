import asyncio
from collections.abc import Coroutine
from enum import Enum
from typing import Literal, Protocol, TypeAlias, TypedDict, Unpack, runtime_checkable

import httpx
import litellm
import openai
from openai import OpenAI
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

# the sync calls answer a page and the async ones a coroutine of one
ReadResult: TypeAlias = BaseModel | Coroutine[object, object, BaseModel]

# the provider names list and alist declare; any other name only reaches the unsupported branch
Provider: TypeAlias = Literal["openai", "azure", "vertex_ai"]


# every keyword argument the four calls take, and the per-call ones list and retrieve add
class ReadValues(TypedDict, total=False):
    api_base: str
    api_key: str | None
    api_version: str | None
    organization: str | None
    timeout: float | str | httpx.Timeout | None
    request_timeout: float | str | None
    max_retries: int | None
    client: OpenAI | None
    after: str | None
    limit: int | None
    custom_llm_provider: str | None
    fine_tuning_job_id: str


class ReadCall(Protocol):
    def __call__(self, **values: Unpack[ReadValues]) -> ReadResult: ...


# litellm leaves the four reads unannotated, so every name on the module answers one typed call
class FineTuningApi(Protocol):
    def __getattr__(self, name: str) -> ReadCall: ...


API: FineTuningApi = litellm


class StructuredTimeout(BaseModel):
    model_config = ConfigDict(extra="forbid")

    connect: float | None = None
    read: float | None = None
    write: float | None = None
    pool: float | None = None


class ReadParams(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_base: str | None = None
    api_key: str | None = None
    api_version: str | None = None
    organization: str | None = None
    timeout: float | str | StructuredTimeout | None = None
    request_timeout: float | str | None = None
    max_retries: int | None = None


class ListCall(ReadParams):
    after: str | None = None
    limit: int | None = None
    provider: Provider = "openai"
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, JsonValue] | None = None


class RetrieveCall(ReadParams):
    fine_tuning_job_id: str
    provider: str | None = "openai"
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, JsonValue] | None = None


class ListStep(BaseModel):
    model_config = ConfigDict(extra="forbid")

    call: ListCall
    supplied_client: bool = False


class RetrieveStep(BaseModel):
    model_config = ConfigDict(extra="forbid")

    call: RetrieveCall
    supplied_client: bool = False


class ListKind(BaseModel):
    list: ListStep


class AListKind(BaseModel):
    alist: ListStep


class RetrieveKind(BaseModel):
    retrieve: RetrieveStep


class ARetrieveKind(BaseModel):
    aretrieve: RetrieveStep


Step = ListKind | AListKind | RetrieveKind | ARetrieveKind


class Failure(BaseModel):
    model_config = ConfigDict(extra="forbid")

    cls: str = Field(serialization_alias="class")
    status: int
    type: str | None = None
    param: str | None = None
    code: str | None = None
    message: str


class StepOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    page: JsonValue | None = None
    job: JsonValue | None = None
    failure: Failure | None = None


class FineTuningReadCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    steps: list[Step]


class Steps(BaseModel):
    model_config = ConfigDict(extra="forbid")

    steps: list[StepOutcome]


class ReadValue(BaseModel):
    model_config = ConfigDict(extra="forbid")

    value: Steps


class Result(BaseModel):
    model_config = ConfigDict(extra="forbid")

    fine_tuning_read: ReadValue


class FineTuningReadRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    fine_tuning_read: FineTuningReadCall

    def serialize(self, api_base: str) -> str:
        steps = [run_step(step, api_base) for step in self.fine_tuning_read.steps]
        result = Result(fine_tuning_read=ReadValue(value=Steps(steps=steps)))
        return result.model_dump_json(by_alias=True, exclude_unset=True)


# the SDK reads a null base as its own default, so the harness base stands in for an absent one
def read_client(step: ListStep | RetrieveStep, harness: str) -> OpenAI | None:
    if not step.supplied_client:
        return None
    base = step.call.api_base if step.call.api_base is not None else harness
    return OpenAI(
        base_url=f"{base}/supplied", api_key="sk-supplied", organization="org-supplied"
    )


def timeout_value(
    timeout: float | str | StructuredTimeout | None,
) -> float | str | httpx.Timeout | None:
    match timeout:
        case StructuredTimeout():
            return httpx.Timeout(
                connect=timeout.connect,
                read=timeout.read,
                write=timeout.write,
                pool=timeout.pool,
            )
        case float() | int():
            return float(timeout)
        case str() | None:
            return timeout


def read_values(
    step: ListStep | RetrieveStep, harness: str, client: OpenAI | None
) -> ReadValues:
    values = ReadValues(
        api_base=step.call.api_base if step.call.api_base is not None else harness,
        api_key=step.call.api_key,
        api_version=step.call.api_version,
        organization=step.call.organization,
        timeout=timeout_value(step.call.timeout),
        request_timeout=step.call.request_timeout,
        max_retries=step.call.max_retries,
        client=client,
        custom_llm_provider=step.call.provider,
    )
    match step:
        case ListStep(call=call):
            values["after"] = call.after
            values["limit"] = call.limit
        case RetrieveStep(call=call):
            values["fine_tuning_job_id"] = call.fine_tuning_job_id
    return values


class ReadOperation(Enum):
    LIST = "list"
    ALIST = "alist"
    RETRIEVE = "retrieve"
    ARETRIEVE = "aretrieve"


# list and alist answer a page, retrieve and aretrieve a job
PAGES: frozenset[ReadOperation] = frozenset({ReadOperation.LIST, ReadOperation.ALIST})


# the openai client raises APIStatusError carrying the status and the body's error fields
def api_failure(error: openai.APIStatusError) -> Failure:
    return Failure(
        cls=type(error).__name__,
        status=error.status_code,
        message=error.message,
        type=error.type,
        param=error.param,
        code=error.code,
    )


@runtime_checkable
class HasDump(Protocol):
    def model_dump_json(self) -> str: ...


def settle(result: ReadResult) -> JsonValue:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                return settle(runner.run(result))
        case HasDump():
            return JSON.validate_json(result.model_dump_json())


def read_outcome(
    step: ListStep | RetrieveStep, harness: str, operation: ReadOperation
) -> StepOutcome:
    values = read_values(step, harness, read_client(step, harness))
    result: ReadResult
    try:
        match operation:
            case ReadOperation.LIST:
                result = API.list_fine_tuning_jobs(**values)
            case ReadOperation.ALIST:
                result = API.alist_fine_tuning_jobs(**values)
            case ReadOperation.RETRIEVE:
                result = API.retrieve_fine_tuning_job(**values)
            case ReadOperation.ARETRIEVE:
                result = API.aretrieve_fine_tuning_job(**values)
        settled = settle(result)
    except openai.APIStatusError as error:
        return StepOutcome(failure=api_failure(error))
    except ValueError as error:
        return StepOutcome(
            failure=Failure(cls=type(error).__name__, status=0, message=str(error))
        )
    if operation in PAGES:
        return StepOutcome(page=settled)
    return StepOutcome(job=settled)


def run_step(step: Step, harness: str) -> StepOutcome:
    match step:
        case ListKind(list=inner):
            return read_outcome(inner, harness, ReadOperation.LIST)
        case AListKind(alist=inner):
            return read_outcome(inner, harness, ReadOperation.ALIST)
        case RetrieveKind(retrieve=inner):
            return read_outcome(inner, harness, ReadOperation.RETRIEVE)
        case ARetrieveKind(aretrieve=inner):
            return read_outcome(inner, harness, ReadOperation.ARETRIEVE)
