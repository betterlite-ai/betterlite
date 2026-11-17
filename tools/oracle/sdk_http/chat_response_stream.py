import asyncio
import base64
import uuid
from collections.abc import Awaitable
from typing import Final, Literal, TypeAlias, TypedDict, cast

import litellm
import openai
from litellm.chat_completions.dispatch import ChatResult
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.responses.litellm_completion_transformation.streaming_iterator import (
    LiteLLMCompletionStreamingIterator as ResponsesIterator,
)
from litellm.types.llms.base import BaseLiteLLMOpenAIResponseObject as BaseResponse
from litellm.types.llms.openai import ResponsesAPIOptionalRequestParams as RequestParams
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter
from pydantic import JsonValue as Json

from messages_sync_stream import Message
from openai_chat import Failure, project_failure

NORMALIZED_UUID: Final[str] = "00000000-0000-0000-0000-000000000000"

# completion returns the chat answer or a coroutine of it, and stream=True settles on the wrapper
CompletionAnswer: TypeAlias = ChatResult | Awaitable[ChatResult]


# A LiteLLM class reads as Any to mypy, so pydantic's field check is the wrapper's boundary
class StreamedAnswer(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    wrapper: CustomStreamWrapper


class StepOutcome(TypedDict):
    events: list[dict[str, Json]]
    end: bool


class RunOutcome(TypedDict):
    steps: list[StepOutcome]
    failure: Failure | None


class Outcome(TypedDict):
    chat_response_stream: dict[Literal["value"], list[RunOutcome]]


class ChatRunSpec(BaseModel):
    model_config = ConfigDict(extra="forbid")

    mode: Literal["sync", "async"]
    steps: list[Literal["next", "drain"]]


class ChatResponseStreamCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    messages: list[Message]
    api_key: str
    timeout_seconds: int
    request_input: str
    responses_api_request: dict[str, Json] = Field(default_factory=dict)
    runs: list[ChatRunSpec]


# The iterator writes sequence_number into __dict__, so it only comes back as an attribute
class Sequenced(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    sequence_number: int | None = None


def normalized_uuid_id(value: str) -> str:
    prefixes = ("chatcmpl-", "resp_", "msg_", "rs_")
    prefix = next((one for one in prefixes if value.startswith(one)), "")
    if not prefix:
        return value
    try:
        uuid.UUID(value.removeprefix(prefix))
    except ValueError:
        return value
    return f"{prefix}{NORMALIZED_UUID}"


def normalized_encoded_id(value: str) -> str:
    try:
        decoded = base64.b64decode(value.removeprefix("resp_"), validate=True).decode()
        provider_part, model_part, response_part = decoded.split(";")
    except ValueError:
        return value
    inner = response_part.removeprefix("response_id:")
    normalized = normalized_uuid_id(inner)
    if inner in (response_part, normalized):
        return value
    assembled = ";".join((provider_part, model_part, f"response_id:{normalized}"))
    return f"resp_{base64.b64encode(assembled.encode()).decode()}"


def normalized_field(key: str, value: Json) -> Json:
    match value:
        case int() if key == "created_at":
            return 0
        case str() if key in ("id", "item_id"):
            if value.startswith("resp_"):
                return normalized_encoded_id(value)
            return normalized_uuid_id(value)
        case list():
            return [normalized_field(key, child) for child in value]
        case dict():
            return {name: normalized_field(name, child) for name, child in value.items()}
        case bool() | float() | str() | int() | None:
            return value


def event_view(event: BaseResponse) -> dict[str, Json]:
    view = TypeAdapter(dict[str, Json]).validate_json(event.model_dump_json())
    sequence = TypeAdapter(Sequenced).validate_python(event).sequence_number
    if sequence is not None:
        view["sequence_number"] = sequence
    return {key: normalized_field(key, value) for key, value in view.items()}


def build_iterator(call: ChatResponseStreamCall, answer: CompletionAnswer) -> ResponsesIterator:
    fields: dict[str, CompletionAnswer] = {"wrapper": answer}
    return ResponsesIterator(
        model=call.model,
        litellm_custom_stream_wrapper=StreamedAnswer.model_validate(fields).wrapper,
        request_input=call.request_input,
        # LiteLLM's request TypedDict carries `metadata: dict[str, Any] | None`
        responses_api_request=cast("RequestParams", call.responses_api_request),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1283
        custom_llm_provider="openai",
    )


def request_options(call: ChatResponseStreamCall, api_base: str) -> dict[str, Json]:
    messages: list[Json] = [
        {"role": message["role"], "content": message["content"]} for message in call.messages
    ]
    return {
        "model": call.model,
        "messages": messages,
        "api_key": call.api_key,
        "api_base": api_base,
        "timeout": call.timeout_seconds,
        "stream": True,
        "stream_options": {"include_usage": True},
        "custom_llm_provider": "openai",
        "num_retries": 0,
    }


def sync_step(iterator: ResponsesIterator, step: Literal["next", "drain"]) -> StepOutcome:
    events: list[dict[str, Json]] = []
    for event in iterator:
        events.append(event_view(event))
        if step == "next":
            return StepOutcome(events=events, end=False)
    return StepOutcome(events=events, end=True)


async def async_step(iterator: ResponsesIterator, step: Literal["next", "drain"]) -> StepOutcome:
    events: list[dict[str, Json]] = []
    while True:
        try:
            events.append(event_view(await iterator.__anext__()))
        except StopAsyncIteration:
            return StepOutcome(events=events, end=True)
        if step == "next":
            return StepOutcome(events=events, end=False)


def sync_run(call: ChatResponseStreamCall, spec: ChatRunSpec, api_base: str) -> RunOutcome:
    try:
        answer: CompletionAnswer = litellm.completion(**request_options(call, api_base))
    except openai.APIError as error:
        return RunOutcome(steps=[], failure=project_failure(error).failure)
    iterator = build_iterator(call, answer)
    return RunOutcome(steps=[sync_step(iterator, step) for step in spec.steps], failure=None)


async def async_run(call: ChatResponseStreamCall, spec: ChatRunSpec, api_base: str) -> RunOutcome:
    try:
        held: Awaitable[CompletionAnswer] = litellm.acompletion(**request_options(call, api_base))
        answer: CompletionAnswer = await held
    except openai.APIError as error:
        return RunOutcome(steps=[], failure=project_failure(error).failure)
    iterator = build_iterator(call, answer)
    steps = [await async_step(iterator, step) for step in spec.steps]
    return RunOutcome(steps=steps, failure=None)


def run_spec(call: ChatResponseStreamCall, spec: ChatRunSpec, api_base: str) -> RunOutcome:
    if spec.mode == "async":
        return asyncio.run(async_run(call, spec, api_base))
    return sync_run(call, spec, api_base)


def run_chat_response_stream(call: ChatResponseStreamCall, api_base: str) -> str:
    runs = [run_spec(call, spec, api_base) for spec in call.runs]
    outcome: Outcome = {"chat_response_stream": {"value": runs}}
    return TypeAdapter(Outcome).dump_json(outcome, by_alias=True, exclude_unset=True).decode()
