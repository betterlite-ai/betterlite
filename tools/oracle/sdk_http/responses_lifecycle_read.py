import asyncio
from collections.abc import Coroutine
from enum import Enum
from typing import Literal, Protocol, TypeAlias, TypedDict, Unpack, runtime_checkable

import httpx
import litellm
import openai
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

from openai_chat import Failure, project_failure

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


# every keyword argument the four SDK calls take; each call reads its own subset
class LifecycleValues(TypedDict, total=False):
    api_base: str | None
    api_key: str | None
    ssl_verify: bool | None
    litellm_call_id: str | None
    metadata: dict[str, JsonValue] | None
    litellm_metadata: dict[str, JsonValue] | None
    custom_llm_provider: str | None
    timeout: float | httpx.Timeout | None
    extra_headers: dict[str, str] | None
    extra_query: dict[str, JsonValue] | None
    extra_body: dict[str, JsonValue] | None
    after: str | None
    before: str | None
    include: list[str] | None
    limit: int | None
    order: Literal["asc", "desc"] | None


# the sync calls answer the response or the item page, the async ones a coroutine of it
LifecycleResult: TypeAlias = (
    BaseModel | dict[str, JsonValue] | Coroutine[None, None, BaseModel | dict[str, JsonValue]]
)


class ReadCall(Protocol):
    def __call__(
        self, response_id: str, **values: Unpack[LifecycleValues]
    ) -> LifecycleResult: ...


# litellm leaves the four reads unannotated, so every name on the module answers one typed call
class LifecycleApi(Protocol):
    def __getattr__(self, name: str) -> ReadCall: ...


API: LifecycleApi = litellm


@runtime_checkable
class HasModelDump(Protocol):
    def model_dump_json(self) -> str: ...


class Answer(Enum):
    RESPONSE = "response"
    ITEMS = "items"


class StructuredTimeout(BaseModel):
    model_config = ConfigDict(extra="forbid")

    connect: float | None = None
    read: float | None = None
    write: float | None = None
    pool: float | None = None


# GenericLiteLLMParams' read fields, plus the identity and metadata B247 logs and B904 rewrites from
class LifecycleParams(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_base: str | None = None
    api_key: str | None = None
    ssl_verify: bool | None = None
    litellm_call_id: str | None = None
    metadata: dict[str, JsonValue] | None = None
    litellm_metadata: dict[str, JsonValue] | None = None
    extra_headers: dict[str, str] | None = None
    timeout: float | StructuredTimeout | None = None


class RetrieveCall(LifecycleParams):
    response_id: str
    custom_llm_provider: str | None = None
    extra_query: dict[str, JsonValue] | None = None
    extra_body: dict[str, JsonValue] | None = None


# absent limit and order take the SDK signature's defaults, while an explicit null passes None on
class InputItemsCall(LifecycleParams):
    response_id: str
    after: str | None = None
    before: str | None = None
    include: list[str] | None = None
    limit: int | None = 20
    order: Literal["asc", "desc"] | None = "desc"
    custom_llm_provider: str | None = None


class RetrieveStep(BaseModel):
    model_config = ConfigDict(extra="forbid")

    call: RetrieveCall


class InputItemsStep(BaseModel):
    model_config = ConfigDict(extra="forbid")

    call: InputItemsCall


class GetResponsesKind(BaseModel):
    get_responses: RetrieveStep


class AgetResponsesKind(BaseModel):
    aget_responses: RetrieveStep


class ListInputItemsKind(BaseModel):
    list_input_items: InputItemsStep


class AlistInputItemsKind(BaseModel):
    alist_input_items: InputItemsStep


Step: TypeAlias = GetResponsesKind | AgetResponsesKind | ListInputItemsKind | AlistInputItemsKind


class Retrieved(BaseModel):
    id: str
    created_at: int
    status: str | None
    model: str | None


class StepOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    retrieved: Retrieved | None = None
    items: JsonValue = None
    failure: Failure | None = None


class ResponsesLifecycleReadCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    steps: list[Step]


class LifecycleOutcome(BaseModel):
    steps: list[StepOutcome]


class LifecycleValue(BaseModel):
    value: LifecycleOutcome


class LifecycleEnvelope(BaseModel):
    responses_lifecycle_read: LifecycleValue


class ResponsesLifecycleReadRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    responses_lifecycle_read: ResponsesLifecycleReadCall

    def serialize(self, api_base: str) -> str:
        steps = [run_step(step, api_base) for step in self.responses_lifecycle_read.steps]
        envelope = LifecycleEnvelope(
            responses_lifecycle_read=LifecycleValue(value=LifecycleOutcome(steps=steps))
        )
        return envelope.model_dump_json(by_alias=True, exclude_unset=True)


# the SDK's timeout is float | httpx.Timeout | None; a null falls back to its request_timeout
def timeout_value(timeout: float | StructuredTimeout | None) -> float | httpx.Timeout | None:
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
        case None:
            return timeout


# the common kwargs every call takes, then the ones only its own step carries
def lifecycle_values(step: RetrieveStep | InputItemsStep, harness: str) -> LifecycleValues:
    call = step.call
    values = LifecycleValues(
        api_base=call.api_base if call.api_base is not None else harness,
        api_key=call.api_key,
        ssl_verify=call.ssl_verify,
        litellm_call_id=call.litellm_call_id,
        metadata=call.metadata,
        litellm_metadata=call.litellm_metadata,
        custom_llm_provider=call.custom_llm_provider,
        timeout=timeout_value(call.timeout),
        extra_headers=call.extra_headers,
    )
    match step:
        case RetrieveStep(call=retrieve):
            values["extra_query"] = retrieve.extra_query
            values["extra_body"] = retrieve.extra_body
        case InputItemsStep(call=items):
            values["after"] = items.after
            values["before"] = items.before
            values["include"] = items.include
            values["limit"] = items.limit
            values["order"] = items.order
    return values


# the wrapper's coroutine is awaited once, so the settled answer is the object or the page itself
def settle(result: LifecycleResult) -> BaseModel | dict[str, JsonValue]:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                return settle(runner.run(result))
        case dict() as page:
            return page
        case HasModelDump():
            return result


def read_outcome(
    step: RetrieveStep | InputItemsStep, harness: str, method: ReadCall, answer: Answer
) -> StepOutcome:
    try:
        settled = settle(method(step.call.response_id, **lifecycle_values(step, harness)))
    except openai.APIError as error:
        return StepOutcome(failure=project_failure(error).failure)
    if answer is Answer.RESPONSE:
        match settled:
            case HasModelDump():
                return StepOutcome(
                    retrieved=Retrieved.model_validate_json(settled.model_dump_json())
                )
            case dict() as body:
                return StepOutcome(retrieved=Retrieved.model_validate(body))
    return StepOutcome(items=JSON.validate_python(settled))


def run_step(step: Step, harness: str) -> StepOutcome:
    match step:
        case GetResponsesKind(get_responses=inner):
            return read_outcome(inner, harness, API.get_responses, Answer.RESPONSE)
        case AgetResponsesKind(aget_responses=inner):
            return read_outcome(inner, harness, API.aget_responses, Answer.RESPONSE)
        case ListInputItemsKind(list_input_items=inner):
            return read_outcome(inner, harness, API.list_input_items, Answer.ITEMS)
        case AlistInputItemsKind(alist_input_items=inner):
            return read_outcome(inner, harness, API.alist_input_items, Answer.ITEMS)
