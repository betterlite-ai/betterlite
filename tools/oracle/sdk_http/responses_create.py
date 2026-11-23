import asyncio
import json
from collections.abc import Coroutine
from typing import Final, Literal, Protocol, TypeAlias, cast

import litellm
import openai
from litellm.types.llms.openai import ResponsesAPIResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Mode = Literal["sync", "async"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

CreateResult: TypeAlias = ResponsesAPIResponse | Coroutine[object, object, ResponsesAPIResponse]


# litellm's @client decorator erases both wrapper signatures, so the protocol names the calls
class ResponsesCalls(Protocol):
    def responses(self, **kwargs: JsonValue) -> CreateResult: ...

    def aresponses(self, **kwargs: JsonValue) -> CreateResult: ...


CREATES: Final[ResponsesCalls] = cast(ResponsesCalls, litellm)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L1127-L1149


class CreateRun(BaseModel):
    model_config = ConfigDict(extra="forbid")

    mode: Mode
    model: str
    input: JsonValue
    stream: bool | None = None
    temperature: float | None = None
    text_format: JsonValue = None
    service_tier: str | None = None
    safety_identifier: str | None = None
    extra_headers: dict[str, JsonValue] | None = None
    extra_query: JsonValue = None
    timeout_seconds: float | None = None
    kwargs: dict[str, JsonValue] = Field(default_factory=dict)


class ResponsesCreateCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[CreateRun]


def call_kwargs(run: CreateRun, api_base: str) -> dict[str, JsonValue]:
    values: dict[str, JsonValue] = {
        "model": run.model,
        "input": run.input,
        "stream": run.stream,
        "temperature": run.temperature,
        "text_format": run.text_format,
        "service_tier": run.service_tier,
        "safety_identifier": run.safety_identifier,
        "extra_headers": run.extra_headers,
        "extra_query": run.extra_query,
        "timeout": run.timeout_seconds,
        "api_base": api_base,
    }
    values.update(run.kwargs)
    return values


def called(run: CreateRun, api_base: str) -> CreateResult:
    kwargs = call_kwargs(run, api_base)
    if run.mode == "async":
        return CREATES.aresponses(**kwargs)
    return CREATES.responses(**kwargs)


def settle(result: CreateResult) -> JsonValue:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                settled = runner.run(result)
            return JSON.validate_json(settled.model_dump_json())
        case object():
            return JSON.validate_json(result.model_dump_json())


def status_code(error: Exception) -> int:
    match error:
        case openai.APIStatusError():
            return error.status_code
        case openai.APIConnectionError():
            return 500
        case object():
            return 0


# litellm builds its exceptions without a body, so the SDK reads no type, param or code
def failure(error: Exception) -> dict[str, JsonValue]:
    return {
        "class": type(error).__name__,
        "status": status_code(error),
        "type": None,
        "param": None,
        "code": None,
        "message": str(error),
    }


def run_outcome(run: CreateRun, api_base: str) -> dict[str, JsonValue]:
    try:
        result = settle(called(run, api_base))
    except (ValueError, openai.OpenAIError) as error:
        return {"result": None, "failure": failure(error)}
    return {"result": result, "failure": None}


class ResponsesCreateRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    responses_create: ResponsesCreateCall

    def serialize(self, api_base: str) -> str:
        runs: list[JsonValue] = [run_outcome(run, api_base) for run in self.responses_create.runs]
        outcome: JsonValue = {"responses_create": {"value": {"runs": runs}}}
        return json.dumps(outcome)
