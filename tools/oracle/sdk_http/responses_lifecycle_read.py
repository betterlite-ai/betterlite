import asyncio
from collections.abc import Coroutine
from typing import Literal, Protocol, TypeAlias, TypedDict, Unpack

import httpx
import litellm
import openai
from litellm.types.llms.openai import ResponsesAPIResponse
from pydantic import BaseModel, ConfigDict, JsonValue

from openai_chat import Failure, optional_text, status_code


# the values both retrieve entries read; the four input-items arguments come on their own dict
class RetrieveValues(TypedDict, total=False):
    api_base: str | None
    api_key: str | None
    ssl_verify: bool | None
    litellm_call_id: str | None
    metadata: dict[str, JsonValue] | None
    litellm_metadata: dict[str, JsonValue] | None
    extra_headers: dict[str, str] | None
    extra_query: dict[str, JsonValue] | None
    extra_body: dict[str, JsonValue] | None
    timeout: float | httpx.Timeout | None
    custom_llm_provider: str | None


class ItemsValues(TypedDict, total=False):
    api_base: str | None
    api_key: str | None
    ssl_verify: bool | None
    litellm_call_id: str | None
    metadata: dict[str, JsonValue] | None
    litellm_metadata: dict[str, JsonValue] | None
    after: str | None
    before: str | None
    include: list[str] | None
    limit: int | None
    order: Literal["asc", "desc"] | None
    extra_headers: dict[str, str] | None
    timeout: float | httpx.Timeout | None
    custom_llm_provider: str | None


# the four SDK entries are unannotated **kwargs functions, so each name is declared as the call it answers
class LifecycleFacade(Protocol):
    def get_responses(
        self, response_id: str, **values: Unpack[RetrieveValues]
    ) -> ResponsesAPIResponse | Coroutine[object, object, ResponsesAPIResponse]: ...

    def aget_responses(
        self, response_id: str, **values: Unpack[RetrieveValues]
    ) -> Coroutine[object, object, ResponsesAPIResponse]: ...

    def list_input_items(
        self, response_id: str, **values: Unpack[ItemsValues]
    ) -> dict[str, JsonValue] | Coroutine[object, object, dict[str, JsonValue]]: ...

    def alist_input_items(
        self, response_id: str, **values: Unpack[ItemsValues]
    ) -> Coroutine[object, object, dict[str, JsonValue]]: ...


FACADE: LifecycleFacade = litellm


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


class RetrieveCall(LifecycleParams):
    response_id: str
    custom_llm_provider: str | None = None
    extra_headers: dict[str, str] | None = None
    extra_query: dict[str, JsonValue] | None = None
    extra_body: dict[str, JsonValue] | None = None
    timeout: float | StructuredTimeout | None = None


# the absent limit and order are the SDK signature's own defaults, while an explicit null passes None on
class InputItemsCall(LifecycleParams):
    response_id: str
    after: str | None = None
    before: str | None = None
    include: list[str] | None = None
    limit: int | None = 20
    order: Literal["asc", "desc"] | None = "desc"
    custom_llm_provider: str | None = None
    extra_headers: dict[str, str] | None = None
    timeout: float | StructuredTimeout | None = None


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
    model_config = ConfigDict(extra="forbid")

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
            responses_lifecycle_read=LifecycleValue(
                value=LifecycleOutcome(steps=steps)
            )
        )
        return envelope.model_dump_json(by_alias=True, exclude_unset=True)


# the SDK's timeout is float | httpx.Timeout | None; a null and a zero both fall back to its request_timeout
def timeout_value(timeout: float | StructuredTimeout | None) -> float | httpx.Timeout | None:
    match timeout:
        case StructuredTimeout():
            return httpx.Timeout(
                connect=timeout.connect,
                read=timeout.read,
                write=timeout.write,
                pool=timeout.pool,
            )
        case float():
            return timeout
        case None:
            return None


def retrieve_values(call: RetrieveCall, harness: str) -> RetrieveValues:
    return RetrieveValues(
        api_base=call.api_base if call.api_base is not None else harness,
        api_key=call.api_key,
        ssl_verify=call.ssl_verify,
        litellm_call_id=call.litellm_call_id,
        metadata=call.metadata,
        litellm_metadata=call.litellm_metadata,
        extra_headers=call.extra_headers,
        extra_query=call.extra_query,
        extra_body=call.extra_body,
        timeout=timeout_value(call.timeout),
        custom_llm_provider=call.custom_llm_provider,
    )


def items_values(call: InputItemsCall, harness: str) -> ItemsValues:
    return ItemsValues(
        api_base=call.api_base if call.api_base is not None else harness,
        api_key=call.api_key,
        ssl_verify=call.ssl_verify,
        litellm_call_id=call.litellm_call_id,
        metadata=call.metadata,
        litellm_metadata=call.litellm_metadata,
        after=call.after,
        before=call.before,
        include=call.include,
        limit=call.limit,
        order=call.order,
        extra_headers=call.extra_headers,
        timeout=timeout_value(call.timeout),
        custom_llm_provider=call.custom_llm_provider,
    )


# the sync entry answers the object and the async one a coroutine of it; the wrapper awaits it once
def settle_response(
    result: ResponsesAPIResponse | Coroutine[object, object, ResponsesAPIResponse],
) -> ResponsesAPIResponse:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                return settle_response(runner.run(result))
        case ResponsesAPIResponse():
            return result


def settle_items(
    result: dict[str, JsonValue] | Coroutine[object, object, dict[str, JsonValue]],
) -> dict[str, JsonValue]:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                return settle_items(runner.run(result))
        case dict():
            return result


def project_failure(error: openai.APIError) -> StepOutcome:
    fields: dict[str, str | int | None] = {
        "cls": type(error).__name__,
        "status": status_code(error),
        "message": error.message,
    }
    for name in ("type", "param", "code"):
        fields.update(optional_text(error, name))
    return StepOutcome(failure=Failure.model_validate(fields))


def project_response(response: ResponsesAPIResponse) -> StepOutcome:
    return StepOutcome(
        retrieved=Retrieved(
            id=response.id,
            created_at=response.created_at,
            status=response.status,
            model=response.model,
        )
    )


def run_retrieve(step: RetrieveStep, harness: str, asynchronous: bool) -> StepOutcome:
    values = retrieve_values(step.call, harness)
    try:
        result = (
            FACADE.aget_responses(step.call.response_id, **values)
            if asynchronous
            else FACADE.get_responses(step.call.response_id, **values)
        )
        response = settle_response(result)
    except openai.APIError as error:
        return project_failure(error)
    return project_response(response)


def run_items(step: InputItemsStep, harness: str, asynchronous: bool) -> StepOutcome:
    values = items_values(step.call, harness)
    try:
        result = (
            FACADE.alist_input_items(step.call.response_id, **values)
            if asynchronous
            else FACADE.list_input_items(step.call.response_id, **values)
        )
        listed = settle_items(result)
    except openai.APIError as error:
        return project_failure(error)
    return StepOutcome(items=listed)


def run_step(step: Step, harness: str) -> StepOutcome:
    match step:
        case GetResponsesKind(get_responses=inner):
            return run_retrieve(inner, harness, False)
        case AgetResponsesKind(aget_responses=inner):
            return run_retrieve(inner, harness, True)
        case ListInputItemsKind(list_input_items=inner):
            return run_items(inner, harness, False)
        case AlistInputItemsKind(alist_input_items=inner):
            return run_items(inner, harness, True)
