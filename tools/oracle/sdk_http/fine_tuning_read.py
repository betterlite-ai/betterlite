import asyncio
from collections.abc import Coroutine
from enum import Enum
from typing import TypeAlias

import httpx
import litellm
import openai
from openai import OpenAI
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

# list answers a page and retrieve a job, and an async flag makes either one return a coroutine
ModelResult: TypeAlias = BaseModel | Coroutine[object, object, BaseModel]


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
    provider: str | None = "openai"
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
        case float() | str() | None:
            return timeout


def arguments(
    call: ReadParams, harness: str, client: OpenAI | None
) -> dict[str, object]:
    return {
        "api_base": call.api_base if call.api_base is not None else harness,
        "api_key": call.api_key,
        "api_version": call.api_version,
        "organization": call.organization,
        "timeout": timeout_value(call.timeout),
        "request_timeout": call.request_timeout,
        "max_retries": call.max_retries,
        "client": client,
    }


class ReadOperation(Enum):
    LIST = "list"
    ALIST = "alist"
    RETRIEVE = "retrieve"
    ARETRIEVE = "aretrieve"


def read_arguments(
    step: ListStep | RetrieveStep, harness: str, client: OpenAI | None
) -> dict[str, object]:
    values = arguments(step.call, harness, client)
    match step:
        case ListStep(call=call):
            values.update(
                {
                    "after": call.after,
                    "limit": call.limit,
                    "custom_llm_provider": call.provider,
                    "extra_headers": call.extra_headers,
                    "extra_body": call.extra_body,
                }
            )
        case RetrieveStep(call=call):
            values.update(
                {
                    "fine_tuning_job_id": call.fine_tuning_job_id,
                    "custom_llm_provider": call.provider,
                    "extra_headers": call.extra_headers,
                    "extra_body": call.extra_body,
                }
            )
    return values


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


def settle(result: ModelResult) -> JsonValue:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                settled = runner.run(result)
            return JSON.validate_json(settled.model_dump_json())
        case BaseModel():
            return JSON.validate_json(result.model_dump_json())


def request(operation: ReadOperation, values: dict[str, object]) -> ModelResult:
    result: ModelResult
    match operation:
        case ReadOperation.LIST:
            result = litellm.list_fine_tuning_jobs(**values)
        case ReadOperation.ALIST:
            result = litellm.alist_fine_tuning_jobs(**values)
        case ReadOperation.RETRIEVE:
            result = litellm.retrieve_fine_tuning_job(**values)
        case ReadOperation.ARETRIEVE:
            result = litellm.aretrieve_fine_tuning_job(**values)
    return result


def read_outcome(
    step: ListStep | RetrieveStep, harness: str, operation: ReadOperation
) -> StepOutcome:
    values = read_arguments(step, harness, read_client(step, harness))
    try:
        settled = settle(request(operation, values))
    except openai.APIStatusError as error:
        return StepOutcome(failure=api_failure(error))
    except ValueError as error:
        return StepOutcome(
            failure=Failure(cls=type(error).__name__, status=0, message=str(error))
        )
    match operation:
        case ReadOperation.LIST | ReadOperation.ALIST:
            return StepOutcome(page=settled)
        case ReadOperation.RETRIEVE | ReadOperation.ARETRIEVE:
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
