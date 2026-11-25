import asyncio
import json
from collections.abc import Coroutine
from datetime import UTC, datetime
from typing import Final, TypeAlias

import openai
from litellm.completion_extras import responses_api_bridge
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.types.utils import CallTypes, ModelResponse, ModelResponseStream
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

# main.py hands a chat call whose model is served by the Responses API to this singleton: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L5790-L5814
BRIDGE: Final = responses_api_bridge

Settled: TypeAlias = ModelResponse | CustomStreamWrapper


class BridgeRun(BaseModel):
    model_config = ConfigDict(extra="forbid")
    acompletion: JsonValue = False
    kwargs: dict[str, JsonValue] = Field(default_factory=dict)


class ResponseChatDispatchCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[BridgeRun]


# StreamingChoices and Delta hide their fields in pydantic's extras, so a typed view reads them
class StreamDelta(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    content: str | None = None


class StreamChoice(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    delta: StreamDelta = Field(default_factory=StreamDelta)
    finish_reason: str | None = None


# Only callbacks read the logging object; the transform gets the response model itself
def logging_object() -> Logging:
    return Logging(
        model="", messages=None, stream=False, call_type=CallTypes.completion,
        start_time=datetime.now(UTC), litellm_call_id="call-1", function_id="fn-1",
        litellm_trace_id="trace-1",
    )


# A run's empty placeholders become the two SDK objects, and the mock origin joins the SDK params
def call_kwargs(run: BridgeRun, api_base: str) -> dict[str, JsonValue | ModelResponse | Logging]:
    values: dict[str, JsonValue | ModelResponse | Logging] = {**run.kwargs}
    if values.get("model_response") == {}:
        values["model_response"] = ModelResponse()
    if values.get("logging_obj") == {}:
        values["logging_obj"] = logging_object()
    values["acompletion"] = run.acompletion
    match values.get("litellm_params"):
        case dict() as params:
            params["api_base"] = api_base
        case None | bool() | int() | float() | str() | list() | ModelResponse() | Logging():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/handler.py#L107-L155
            pass
    return values


# litellm's bridge takes untyped kwargs and settles on a response, a stream or a coroutine of either
def call_bridge(run: BridgeRun, api_base: str) -> Settled | Coroutine[None, None, Settled]:
    return BRIDGE.completion(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/handler.py#L157-L164
        **call_kwargs(run, api_base)
    )


def complete(run: BridgeRun, api_base: str) -> ModelResponse | CustomStreamWrapper:
    match call_bridge(run, api_base):
        case ModelResponse() | CustomStreamWrapper() as settled:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/handler.py#L157-L164
            return settled
        case object():
            raise TypeError("a sync completion returned a coroutine")


async def complete_async(run: BridgeRun, api_base: str) -> ModelResponse | CustomStreamWrapper:
    match call_bridge(run, api_base):
        case Coroutine() as pending:
            return await pending
        case object():
            raise TypeError("an async completion returned a settled response")


def chat_view(response: ModelResponse) -> dict[str, JsonValue]:
    choice = response.choices[0] if response.choices else None
    message = choice.message if choice is not None else None
    return {
        "id": response.id, "model": response.model, "object": response.object,
        "role": message.role if message is not None else None,
        "content": message.content if message is not None else None,
        "finish_reason": choice.finish_reason if choice is not None else None,
    }


def stream_view(chunks: list[ModelResponseStream]) -> dict[str, JsonValue]:
    kept: list[JsonValue] = []
    identifiers: list[str] = []
    for chunk in chunks:
        if not chunk.choices:
            continue
        choice = StreamChoice.model_validate(chunk.choices[0])
        spoken: str | None = choice.delta.content
        if spoken == "":
            spoken = None
        if spoken is None and choice.finish_reason is None:
            continue
        view: dict[str, JsonValue] = {"content": spoken, "finish_reason": choice.finish_reason}
        kept.append(view)
        if spoken is not None:
            identifiers.append(chunk.id)
    first = identifiers[0] if identifiers else None
    stable = first is not None and all(identifier == first for identifier in identifiers)
    return {"chunks": kept, "stable_id": stable}


def settled_sync(run: BridgeRun, api_base: str) -> ModelResponse | dict[str, JsonValue]:
    match complete(run, api_base):
        case ModelResponse() as response:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/handler.py#L218-L220
            return response
        case CustomStreamWrapper() as stream:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/handler.py#L251-L256
            return stream_view(list(stream))


async def settled_async(run: BridgeRun, api_base: str) -> ModelResponse | dict[str, JsonValue]:
    match await complete_async(run, api_base):
        case ModelResponse() as response:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/handler.py#L314-L316
            return response
        case CustomStreamWrapper() as stream:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/handler.py#L347-L353
            return stream_view([chunk async for chunk in stream])


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
        "class": type(error).__name__, "status": status_code(error),
        "type": None, "param": None, "code": None, "message": str(error),
    }


def run_outcome(run: BridgeRun, api_base: str) -> dict[str, JsonValue]:
    try:
        if run.acompletion is True:
            settled = asyncio.run(settled_async(run, api_base))
        else:
            settled = settled_sync(run, api_base)
    except (ValueError, openai.OpenAIError) as error:
        return {"result": None, "stream": None, "failure": failure(error)}
    match settled:
        case ModelResponse() as response:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/handler.py#L204-L217
            return {"result": chat_view(response), "stream": None, "failure": None}
        case object():
            return {"result": None, "stream": JSON.validate_python(settled), "failure": None}


class ResponseChatDispatchRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    response_chat_dispatch: ResponseChatDispatchCall

    def serialize(self, api_base: str) -> str:
        runs: list[JsonValue] = [
            JSON.validate_python(run_outcome(run, api_base))
            for run in self.response_chat_dispatch.runs
        ]
        outcome: JsonValue = {"response_chat_dispatch": {"value": {"runs": runs}}}
        return json.dumps(outcome)
