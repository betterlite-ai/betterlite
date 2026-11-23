import asyncio
from typing import Final, TypeAlias

import litellm
from litellm.types.decisions import DecisionsResponse, OpenAIDecisionResponse
from openai import APIConnectionError, APIError, APIStatusError, APITimeoutError
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

MOCK: Final = "{mock}"
# litellm.APIConnectionError sets this in its __init__, the way every connection error reports 500
CONNECTION_STATUS: Final = 500
# litellm.Timeout is an openai.APITimeoutError, and every timeout LiteLLM raises carries 408
TIMEOUT_STATUS: Final = 408

Answer: TypeAlias = DecisionsResponse | OpenAIDecisionResponse

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
KWARGS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class Run(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    input: JsonValue = None
    questions: JsonValue = None
    safety_identifier: str | None = None
    custom_llm_provider: str | None = None
    api_key: str | None = None
    api_base: str | None = None
    timeout: float | None = None
    extra_headers: dict[str, str] | None = None
    asynchronous: bool = False
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
    decisions_send: OutcomeValue


class DecisionsSendCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[Run]


def substituted_kwargs(kwargs: dict[str, JsonValue], api_base: str) -> dict[str, JsonValue]:
    document = KWARGS.dump_json(kwargs).decode().replace(MOCK, api_base)
    return KWARGS.validate_json(document)


def failure_of(error: APIError, status: int | None) -> Failure:
    return Failure(
        class_name=type(error).__name__,
        status=status,
        kind=error.type,
        param=error.param,
        code=error.code,
        message=error.message,
    )


async def answered(run: Run, kwargs: dict[str, JsonValue], api_base: str | None) -> JsonValue:
    if run.asynchronous:
        answer: Answer = await litellm.adecisions(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/decisions/main.py#L75-L100
            model=run.model,
            input=run.input,
            questions=run.questions,
            safety_identifier=run.safety_identifier,
            api_key=run.api_key,
            api_base=api_base,
            timeout=run.timeout,
            custom_llm_provider=run.custom_llm_provider,
            extra_headers=run.extra_headers,
            **kwargs,
        )
        return JSON.validate_json(answer.model_dump_json())
    blocking: Answer = litellm.decisions(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/decisions/main.py#L103-L128
        model=run.model,
        input=run.input,
        questions=run.questions,
        safety_identifier=run.safety_identifier,
        api_key=run.api_key,
        api_base=api_base,
        timeout=run.timeout,
        custom_llm_provider=run.custom_llm_provider,
        extra_headers=run.extra_headers,
        **kwargs,
    )
    return JSON.validate_json(blocking.model_dump_json())


async def run_outcome(run: Run, api_base: str) -> RunOutcome:
    kwargs = substituted_kwargs(run.kwargs, api_base)
    base = None if run.api_base is None else run.api_base.replace(MOCK, api_base)
    try:
        result = await answered(run, kwargs, base)
    except APIStatusError as error:
        return RunOutcome(failure=failure_of(error, error.status_code))
    except APITimeoutError as error:
        return RunOutcome(failure=failure_of(error, TIMEOUT_STATUS))
    except APIConnectionError as error:
        return RunOutcome(failure=failure_of(error, CONNECTION_STATUS))
    return RunOutcome(result=result)


async def collect(call: DecisionsSendCall, api_base: str) -> Outcome:
    runs = [await run_outcome(run, api_base) for run in call.runs]
    return Outcome(runs=runs)


class DecisionsSendRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    decisions_send: DecisionsSendCall

    def serialize(self, api_base: str) -> str:
        outcome = asyncio.run(collect(self.decisions_send, api_base))
        envelope = Envelope(decisions_send=OutcomeValue(value=outcome))
        return envelope.model_dump_json(by_alias=True)
