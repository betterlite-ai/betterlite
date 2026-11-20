import asyncio
import json
from collections.abc import Coroutine
from typing import Final, Literal, Protocol, TypeAlias, cast

import litellm
import openai
from litellm.types.llms.openai import ResponsesAPIResponse
from openai.types.responses.response_input_param import ResponseInputParam
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Mode = Literal["sync", "async"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
INPUT: TypeAdapter[str | ResponseInputParam] = TypeAdapter(str | ResponseInputParam)

CompactResult: TypeAlias = ResponsesAPIResponse | Coroutine[object, object, ResponsesAPIResponse]


# litellm's @client decorator erases both wrapper signatures, so the protocol names the calls
class CompactCalls(Protocol):
    def compact_responses(self, **kwargs: JsonValue) -> CompactResult: ...

    def acompact_responses(self, **kwargs: JsonValue) -> CompactResult: ...


COMPACTS: Final[CompactCalls] = cast(CompactCalls, litellm)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L2080-L2159


class CompactRun(BaseModel):
    model_config = ConfigDict(extra="forbid")

    mode: Mode
    model: str
    input: JsonValue
    instructions: str | None = None
    previous_response_id: str | None = None
    custom_llm_provider: str | None = None
    extra_headers: dict[str, JsonValue] | None = None
    extra_query: dict[str, JsonValue] | None = None
    extra_body: dict[str, JsonValue] | None = None
    timeout_seconds: float | None = None
    kwargs: dict[str, JsonValue] = Field(default_factory=dict)


class ResponsesCompactCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[CompactRun]


def call_kwargs(run: CompactRun, api_base: str) -> dict[str, JsonValue]:
    values: dict[str, JsonValue] = {
        "input": JSON.validate_python(INPUT.validate_python(run.input)),
        "model": run.model,
        "instructions": run.instructions,
        "previous_response_id": run.previous_response_id,
        "extra_headers": run.extra_headers,
        "extra_query": run.extra_query,
        "extra_body": run.extra_body,
        "timeout": run.timeout_seconds,
        "custom_llm_provider": run.custom_llm_provider,
        "api_base": api_base,
    }
    values.update(run.kwargs)
    return values


def called(run: CompactRun, api_base: str) -> CompactResult:
    kwargs = call_kwargs(run, api_base)
    if run.mode == "async":
        return COMPACTS.acompact_responses(**kwargs)
    return COMPACTS.compact_responses(**kwargs)


def settle(result: CompactResult) -> JsonValue:
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


# the compact wrapper builds its exceptions with body=None, so the SDK reads no type, param or code
def failure(error: Exception) -> dict[str, JsonValue]:
    return {
        "class": type(error).__name__,
        "status": status_code(error),
        "type": None,
        "param": None,
        "code": None,
        "message": str(error),
    }


def run_outcome(run: CompactRun, api_base: str) -> dict[str, JsonValue]:
    try:
        result = settle(called(run, api_base))
    except (ValueError, openai.OpenAIError) as error:
        return {"result": None, "failure": failure(error)}
    return {"result": result, "failure": None}


class ResponsesCompactRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    responses_compact: ResponsesCompactCall

    def serialize(self, api_base: str) -> str:
        runs: list[JsonValue] = [run_outcome(run, api_base) for run in self.responses_compact.runs]
        outcome: JsonValue = {"responses_compact": {"value": {"runs": runs}}}
        return json.dumps(outcome)
