import asyncio
from collections.abc import Coroutine
from typing import Final, Literal, TypeAlias

import litellm
from litellm.types.rerank import RerankResponse
from openai import APIConnectionError, APIStatusError
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

MOCK: Final = "{mock}"
# litellm.APIConnectionError sets this in its __init__, the way every connection error reports 500
CONNECTION_STATUS: Final = 500

Provider: TypeAlias = Literal[
    "cohere",
    "together_ai",
    "deepinfra",
    "fireworks_ai",
    "voyage",
    "watsonx",
]
Document: TypeAlias = str | dict[str, JsonValue]
Work: TypeAlias = RerankResponse | Coroutine[object, object, RerankResponse]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
KWARGS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class Run(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    query: str
    documents: list[Document]
    custom_llm_provider: Provider | None = None
    top_n: int | None = None
    rank_fields: list[str] | None = None
    return_documents: bool | None = None
    max_chunks_per_doc: int | None = None
    kwargs: dict[str, JsonValue] = Field(default_factory=dict)


class Failure(BaseModel):
    model_config = ConfigDict(extra="forbid")

    class_name: str = Field(serialization_alias="class")
    status: int | None
    kind: str | None = Field(default=None, serialization_alias="type")
    param: str | None = None
    code: str | None = None
    message: str


class RunOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    result: JsonValue = None
    failure: Failure | None = None


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[RunOutcome]


class OutcomeValue(BaseModel):
    value: Outcome


class Envelope(BaseModel):
    rerank_call: OutcomeValue


class RerankCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[Run]


def substituted_kwargs(kwargs: dict[str, JsonValue], api_base: str) -> dict[str, JsonValue]:
    document = KWARGS.dump_json(kwargs).decode().replace(MOCK, api_base)
    return KWARGS.validate_json(document)


async def settled(work: Work) -> RerankResponse:
    match work:
        case Coroutine():
            return await work
        case object():
            return work


def status_failure(error: APIStatusError) -> Failure:
    return Failure(
        class_name=type(error).__name__,
        status=error.status_code,
        kind=error.type,
        param=error.param,
        code=error.code,
        message=error.message,
    )


def connection_failure(error: APIConnectionError) -> Failure:
    return Failure(
        class_name=type(error).__name__,
        status=CONNECTION_STATUS,
        kind=error.type,
        param=error.param,
        code=error.code,
        message=error.message,
    )


async def run_outcome(run: Run, api_base: str) -> RunOutcome:
    kwargs = substituted_kwargs(run.kwargs, api_base)
    try:
        work: Work = litellm.arerank(
            model=run.model,
            query=run.query,
            documents=run.documents,
            custom_llm_provider=run.custom_llm_provider,
            top_n=run.top_n,
            rank_fields=run.rank_fields,
            return_documents=run.return_documents,
            max_chunks_per_doc=run.max_chunks_per_doc,
            **kwargs,
        )
        answer: RerankResponse = await settled(work)
    except APIStatusError as error:
        return RunOutcome(failure=status_failure(error))
    except APIConnectionError as error:
        return RunOutcome(failure=connection_failure(error))
    return RunOutcome(result=JSON.validate_json(answer.model_dump_json()))


async def collect(call: RerankCall, api_base: str) -> Outcome:
    runs = [await run_outcome(run, api_base) for run in call.runs]
    return Outcome(runs=runs)


class RerankCallRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    rerank_call: RerankCall

    def serialize(self, api_base: str) -> str:
        outcome = asyncio.run(collect(self.rerank_call, api_base))
        envelope = Envelope(rerank_call=OutcomeValue(value=outcome))
        return envelope.model_dump_json(by_alias=True)
