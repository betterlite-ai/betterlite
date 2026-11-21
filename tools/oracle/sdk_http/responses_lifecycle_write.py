import asyncio
from collections.abc import Coroutine
from typing import Literal, Protocol, TypeAlias, TypedDict, Unpack, cast

import litellm
import openai
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.responses.main import DeleteResponseResult
from pydantic import BaseModel, ConfigDict, Field, JsonValue

Operation: TypeAlias = Literal["delete", "cancel"]
Mode: TypeAlias = Literal["sync", "async"]

DeleteAnswer: TypeAlias = (
    DeleteResponseResult | Coroutine[object, object, DeleteResponseResult]
)
CancelAnswer: TypeAlias = (
    ResponsesAPIResponse | Coroutine[object, object, ResponsesAPIResponse]
)


class Params(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_key: str | None = None
    litellm_call_id: str | None = None
    metadata: JsonValue = None
    litellm_metadata: JsonValue = None


class Run(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Operation
    mode: Mode
    response_id: str
    custom_llm_provider: str | None = None
    extra_headers: dict[str, str] | None = None
    extra_query: dict[str, JsonValue] | None = None
    extra_body: dict[str, JsonValue] | None = None
    timeout: float | None = None
    params: Params = Field(default_factory=Params)


class LifecycleWriteCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[Run]


class CallKwargs(TypedDict):
    response_id: str
    custom_llm_provider: str | None
    api_base: str
    api_key: str | None
    extra_headers: dict[str, str] | None
    extra_query: dict[str, JsonValue] | None
    extra_body: dict[str, JsonValue] | None
    timeout: float | None
    litellm_call_id: str | None
    metadata: JsonValue
    litellm_metadata: JsonValue


class Lifecycle(Protocol):
    def delete_responses(self, **kwargs: Unpack[CallKwargs]) -> DeleteAnswer: ...

    def adelete_responses(self, **kwargs: Unpack[CallKwargs]) -> DeleteAnswer: ...

    def cancel_responses(self, **kwargs: Unpack[CallKwargs]) -> CancelAnswer: ...

    def acancel_responses(self, **kwargs: Unpack[CallKwargs]) -> CancelAnswer: ...


LIFECYCLE = cast(Lifecycle, litellm)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L1457-L1610


class RaisedStatus(BaseModel):
    status_code: int


class Failure(BaseModel):
    class_name: str = Field(serialization_alias="class")
    status: int
    kind: str | None = Field(default=None, serialization_alias="type")
    param: str | None = None
    code: str | None = None
    message: str


class RunOutcome(BaseModel):
    result: JsonValue = None
    error: Failure | None = None


class Outcome(BaseModel):
    runs: list[RunOutcome]


class LifecycleValue(BaseModel):
    value: Outcome


class LifecycleEnvelope(BaseModel):
    responses_lifecycle_write: LifecycleValue


class ResponsesLifecycleWriteRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    responses_lifecycle_write: LifecycleWriteCall

    def serialize(self, api_base: str) -> str:
        runs: list[RunOutcome]
        with asyncio.Runner() as runner:
            runs = [run_one(run, api_base, runner) for run in self.responses_lifecycle_write.runs]
        envelope = LifecycleEnvelope(
            responses_lifecycle_write=LifecycleValue(value=Outcome(runs=runs))
        )
        return envelope.model_dump_json(by_alias=True)


def call_kwargs(run: Run, api_base: str) -> CallKwargs:
    return {
        "response_id": run.response_id,
        "custom_llm_provider": run.custom_llm_provider,
        "api_base": api_base,
        "api_key": run.params.api_key,
        "extra_headers": run.extra_headers,
        "extra_query": run.extra_query,
        "extra_body": run.extra_body,
        "timeout": run.timeout,
        "litellm_call_id": run.params.litellm_call_id,
        "metadata": run.params.metadata,
        "litellm_metadata": run.params.litellm_metadata,
    }


def called_delete(run: Run, api_base: str) -> DeleteAnswer:
    kwargs = call_kwargs(run, api_base)
    if run.mode == "async":
        return LIFECYCLE.adelete_responses(**kwargs)
    return LIFECYCLE.delete_responses(**kwargs)


def called_cancel(run: Run, api_base: str) -> CancelAnswer:
    kwargs = call_kwargs(run, api_base)
    if run.mode == "async":
        return LIFECYCLE.acancel_responses(**kwargs)
    return LIFECYCLE.cancel_responses(**kwargs)


def delete_result(run: Run, api_base: str, runner: asyncio.Runner) -> DeleteResponseResult:
    answer = called_delete(run, api_base)
    match answer:
        case Coroutine():
            return runner.run(answer)
        case object():
            return answer


def cancel_result(run: Run, api_base: str, runner: asyncio.Runner) -> ResponsesAPIResponse:
    answer = called_cancel(run, api_base)
    match answer:
        case Coroutine():
            return runner.run(answer)
        case object():
            return answer


def deleted_value(deleted: DeleteResponseResult) -> JsonValue:
    fields: dict[str, JsonValue] = {
        "id": deleted.id,
        "object": deleted.object,
        "deleted": deleted.deleted,
    }
    return fields


def cancelled_value(reply: ResponsesAPIResponse) -> JsonValue:
    fields: dict[str, JsonValue] = {
        "id": reply.id,
        "created_at": reply.created_at,
        "object": reply.object,
        "status": reply.status,
        "model": reply.model,
    }
    return fields


def result_of(run: Run, api_base: str, runner: asyncio.Runner) -> JsonValue:
    if run.operation == "delete":
        return deleted_value(delete_result(run, api_base, runner))
    return cancelled_value(cancel_result(run, api_base, runner))


def failure(error: openai.APIError) -> Failure:
    match error:
        case openai.APIStatusError():
            return Failure(
                class_name=type(error).__name__,
                status=error.status_code,
                kind=error.type,
                param=error.param,
                code=error.code,
                message=str(error),
            )
        case openai.APIError():
            status = RaisedStatus.model_validate(error, from_attributes=True).status_code
            return Failure(
                class_name=type(error).__name__,
                status=status,
                message=str(error),
            )


def run_one(run: Run, api_base: str, runner: asyncio.Runner) -> RunOutcome:
    try:
        result = result_of(run, api_base, runner)
    except openai.APIError as error:
        return RunOutcome(error=failure(error))
    return RunOutcome(result=result)
