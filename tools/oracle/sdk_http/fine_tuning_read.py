import asyncio
from collections.abc import Coroutine
from typing import TypeAlias

import httpx
import litellm
import openai
from openai import OpenAI
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

MISSING = object()

# list answers a page and retrieve a job, and an async flag makes either one return a coroutine
ModelResult: TypeAlias = BaseModel | Coroutine[object, object, BaseModel]


class ExceptionFieldTypeError(TypeError):
    def __init__(self, name: str) -> None:
        super().__init__(f"exception field {name} is neither text nor null")


class MissingStatusError(TypeError):
    def __init__(self, name: str) -> None:
        super().__init__(f"exception {name} carries no integer status code")


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
    extra_body: dict[str, str] | None = None


class RetrieveCall(ReadParams):
    fine_tuning_job_id: str
    provider: str | None = "openai"
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, str] | None = None


class ListStep(BaseModel):
    model_config = ConfigDict(extra="forbid")

    call: ListCall
    supplied_client: bool = False


class RetrieveStep(BaseModel):
    model_config = ConfigDict(extra="forbid")

    call: RetrieveCall
    supplied_client: bool = False


class ListKind(BaseModel):
    model_config = ConfigDict(extra="forbid")

    list: ListStep


class AListKind(BaseModel):
    model_config = ConfigDict(extra="forbid")

    alist: ListStep


class RetrieveKind(BaseModel):
    model_config = ConfigDict(extra="forbid")

    retrieve: RetrieveStep


class ARetrieveKind(BaseModel):
    model_config = ConfigDict(extra="forbid")

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
def call_base(call: ReadParams, harness: str) -> str:
    return call.api_base if call.api_base is not None else harness


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


# a supplied client carries its own base, key and organization
def read_client(step: ListStep | RetrieveStep, harness: str) -> OpenAI | None:
    if not step.supplied_client:
        return None
    base = call_base(step.call, harness)
    return OpenAI(
        base_url=f"{base}/supplied", api_key="sk-supplied", organization="org-supplied"
    )


def arguments(
    call: ReadParams, harness: str, client: OpenAI | None
) -> dict[str, object]:
    return {
        "api_base": call_base(call, harness),
        "api_key": call.api_key,
        "api_version": call.api_version,
        "organization": call.organization,
        "timeout": timeout_value(call.timeout),
        "request_timeout": call.request_timeout,
        "max_retries": call.max_retries,
        "client": client,
    }


def list_arguments(
    call: ListCall, harness: str, client: OpenAI | None
) -> dict[str, object]:
    values = arguments(call, harness, client)
    values.update(
        {
            "after": call.after,
            "limit": call.limit,
            "custom_llm_provider": call.provider,
            "extra_headers": call.extra_headers,
            "extra_body": call.extra_body,
        }
    )
    return values


def retrieve_arguments(
    call: RetrieveCall, harness: str, client: OpenAI | None
) -> dict[str, object]:
    values = arguments(call, harness, client)
    values.update(
        {
            "fine_tuning_job_id": call.fine_tuning_job_id,
            "custom_llm_provider": call.provider,
            "extra_headers": call.extra_headers,
            "extra_body": call.extra_body,
        }
    )
    return values


# the openai client reads type, param and code off the body's error object
def status_code(error: openai.APIError) -> int:
    value: object = getattr(error, "status_code", MISSING)
    if isinstance(value, int):
        return value
    raise MissingStatusError(type(error).__name__)


def optional_text(error: openai.APIError, name: str) -> dict[str, str | None]:
    value: object = getattr(error, name, MISSING)
    if value is MISSING:
        return {}
    if value is None or isinstance(value, str):
        return {name: value}
    raise ExceptionFieldTypeError(name)


def api_failure(error: openai.APIError) -> Failure:
    fields: dict[str, str | int | None] = {
        "cls": type(error).__name__,
        "status": status_code(error),
        "message": error.message,
    }
    for name in ("type", "param", "code"):
        fields.update(optional_text(error, name))
    return Failure.model_validate(fields)


def plain_failure(error: ValueError) -> Failure:
    return Failure(cls=type(error).__name__, status=0, message=str(error))


def settle(result: ModelResult) -> JsonValue:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                settled = runner.run(result)
            return JSON.validate_json(settled.model_dump_json())
        case BaseModel():
            return JSON.validate_json(result.model_dump_json())


def list_outcome(step: ListStep, harness: str) -> StepOutcome:
    values = list_arguments(step.call, harness, read_client(step, harness))
    try:
        page: ModelResult = litellm.list_fine_tuning_jobs(**values)
        settled = settle(page)
    except openai.APIError as error:
        return StepOutcome(failure=api_failure(error))
    except ValueError as error:
        return StepOutcome(failure=plain_failure(error))
    return StepOutcome(page=settled)


def alist_outcome(step: ListStep, harness: str) -> StepOutcome:
    values = list_arguments(step.call, harness, read_client(step, harness))
    try:
        page: ModelResult = litellm.alist_fine_tuning_jobs(**values)
        settled = settle(page)
    except openai.APIError as error:
        return StepOutcome(failure=api_failure(error))
    except ValueError as error:
        return StepOutcome(failure=plain_failure(error))
    return StepOutcome(page=settled)


def retrieve_outcome(step: RetrieveStep, harness: str) -> StepOutcome:
    values = retrieve_arguments(step.call, harness, read_client(step, harness))
    try:
        job: ModelResult = litellm.retrieve_fine_tuning_job(**values)
        settled = settle(job)
    except openai.APIError as error:
        return StepOutcome(failure=api_failure(error))
    except ValueError as error:
        return StepOutcome(failure=plain_failure(error))
    return StepOutcome(job=settled)


def aretrieve_outcome(step: RetrieveStep, harness: str) -> StepOutcome:
    values = retrieve_arguments(step.call, harness, read_client(step, harness))
    try:
        job: ModelResult = litellm.aretrieve_fine_tuning_job(**values)
        settled = settle(job)
    except openai.APIError as error:
        return StepOutcome(failure=api_failure(error))
    except ValueError as error:
        return StepOutcome(failure=plain_failure(error))
    return StepOutcome(job=settled)


def run_step(step: Step, harness: str) -> StepOutcome:
    match step:
        case ListKind(list=inner):
            return list_outcome(inner, harness)
        case AListKind(alist=inner):
            return alist_outcome(inner, harness)
        case RetrieveKind(retrieve=inner):
            return retrieve_outcome(inner, harness)
        case ARetrieveKind(aretrieve=inner):
            return aretrieve_outcome(inner, harness)
