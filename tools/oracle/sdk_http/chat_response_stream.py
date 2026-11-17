import asyncio
import base64
import uuid
from typing import Final, Literal, TypedDict

import litellm
import openai
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.responses.litellm_completion_transformation.streaming_iterator import (
    LiteLLMCompletionStreamingIterator as ResponsesIterator,
)
from litellm.types.llms.base import BaseLiteLLMOpenAIResponseObject as BaseResponse
from litellm.types.llms.openai import ResponsesAPIOptionalRequestParams as RequestParams
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter
from pydantic import JsonValue as Json

from messages_sync_stream import Message
from openai_chat import Failure, project_failure

NORMALIZED_UUID: Final[str] = "00000000-0000-0000-0000-000000000000"
EVENT: Final[TypeAdapter[dict[str, Json]]] = TypeAdapter(dict[str, Json])


class StreamedResponseError(TypeError):
    def __init__(self) -> None:
        super().__init__("litellm.completion returned a response instead of a stream")


class MissingFailureError(TypeError):
    def __init__(self) -> None:
        super().__init__("the projected failure is missing")


class StepOutcome(TypedDict):
    events: list[dict[str, Json]]
    end: bool


class RunOutcome(TypedDict):
    steps: list[StepOutcome]
    failure: Failure | None


class Outcome(TypedDict):
    chat_response_stream: dict[Literal["value"], dict[Literal["runs"], list[RunOutcome]]]


class ResponsesRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    instructions: str | None = None
    temperature: float | None = None
    top_p: float | None = None
    metadata: dict[str, str] | None = None


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
    responses_api_request: ResponsesRequest = Field(default_factory=ResponsesRequest)
    runs: list[ChatRunSpec]


# The iterator writes sequence_number into __dict__, so it only comes back as an attribute
class Sequenced(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    sequence_number: int | None = None


SEQUENCE: Final[TypeAdapter[Sequenced]] = TypeAdapter(Sequenced)


def request_params(request: ResponsesRequest) -> RequestParams:
    params: RequestParams = {}
    if request.instructions is not None:
        params["instructions"] = request.instructions
    if request.temperature is not None:
        params["temperature"] = request.temperature
    if request.top_p is not None:
        params["top_p"] = request.top_p
    if request.metadata is not None:
        params["metadata"] = dict(request.metadata)
    return params


def failure_of(error: openai.APIError) -> Failure:
    failure = project_failure(error).failure
    if failure is None:
        raise MissingFailureError
    return failure


def normalized_uuid_id(value: str) -> str:
    for prefix in ("chatcmpl-", "resp_", "msg_", "rs_"):
        if not value.startswith(prefix):
            continue
        try:
            uuid.UUID(value.removeprefix(prefix))
        except ValueError:
            return value
        return f"{prefix}{NORMALIZED_UUID}"
    return value


def normalized_encoded_id(value: str) -> str:
    try:
        text = base64.b64decode(value.removeprefix("resp_"), validate=True).decode()
        provider_part, model_part, response_part = text.split(";")
    except ValueError:
        return value
    response_prefix = "response_id:"
    if not response_part.startswith(response_prefix):
        return value
    inner = response_part.removeprefix(response_prefix)
    normalized = normalized_uuid_id(inner)
    if normalized == inner:
        return value
    assembled = ";".join((provider_part, model_part, f"{response_prefix}{normalized}"))
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
    view = EVENT.validate_json(event.model_dump_json())
    sequence = SEQUENCE.validate_python(event).sequence_number
    if sequence is not None:
        view["sequence_number"] = sequence
    return {key: normalized_field(key, value) for key, value in view.items()}


def build_iterator(call: ChatResponseStreamCall, stream: CustomStreamWrapper) -> ResponsesIterator:
    return ResponsesIterator(
        model=call.model,
        litellm_custom_stream_wrapper=stream,
        request_input=call.request_input,
        responses_api_request=request_params(call.responses_api_request),
        custom_llm_provider="openai",
    )


def sync_stream(call: ChatResponseStreamCall, api_base: str) -> CustomStreamWrapper:
    stream: ModelResponse | CustomStreamWrapper = litellm.completion(
        model=call.model,
        messages=[
            {"role": message["role"], "content": message["content"]}
            for message in call.messages
        ],
        api_key=call.api_key,
        api_base=api_base,
        timeout=call.timeout_seconds,
        stream=True,
        stream_options={"include_usage": True},
        custom_llm_provider="openai",
        num_retries=0,
    )
    match stream:
        case CustomStreamWrapper():
            return stream
        case ModelResponse():
            raise StreamedResponseError


async def async_stream(call: ChatResponseStreamCall, api_base: str) -> CustomStreamWrapper:
    stream: ModelResponse | CustomStreamWrapper = await litellm.acompletion(
        model=call.model,
        messages=[
            {"role": message["role"], "content": message["content"]}
            for message in call.messages
        ],
        api_key=call.api_key,
        api_base=api_base,
        timeout=call.timeout_seconds,
        stream=True,
        stream_options={"include_usage": True},
        custom_llm_provider="openai",
        num_retries=0,
    )
    match stream:
        case CustomStreamWrapper():
            return stream
        case ModelResponse():
            raise StreamedResponseError


def sync_step(iterator: ResponsesIterator, step: Literal["next", "drain"]) -> StepOutcome:
    events: list[dict[str, Json]] = []
    end = False
    while True:
        try:
            events.append(event_view(next(iterator)))
        except StopIteration:
            end = True
            break
        if step == "next":
            break
    return StepOutcome(events=events, end=end)


async def async_step(iterator: ResponsesIterator, step: Literal["next", "drain"]) -> StepOutcome:
    events: list[dict[str, Json]] = []
    end = False
    while True:
        try:
            event = await iterator.__anext__()
        except StopAsyncIteration:
            end = True
            break
        events.append(event_view(event))
        if step == "next":
            break
    return StepOutcome(events=events, end=end)


def run_sync(
    call: ChatResponseStreamCall, spec: ChatRunSpec, stream: CustomStreamWrapper
) -> RunOutcome:
    iterator = build_iterator(call, stream)
    return RunOutcome(steps=[sync_step(iterator, step) for step in spec.steps], failure=None)


async def run_async(
    call: ChatResponseStreamCall, spec: ChatRunSpec, stream: CustomStreamWrapper
) -> RunOutcome:
    iterator = build_iterator(call, stream)
    return RunOutcome(steps=[await async_step(iterator, step) for step in spec.steps], failure=None)


def sync_run(call: ChatResponseStreamCall, spec: ChatRunSpec, api_base: str) -> RunOutcome:
    try:
        stream = sync_stream(call, api_base)
    except openai.APIError as error:
        return RunOutcome(steps=[], failure=failure_of(error))
    return run_sync(call, spec, stream)


async def async_run(call: ChatResponseStreamCall, spec: ChatRunSpec, api_base: str) -> RunOutcome:
    try:
        stream = await async_stream(call, api_base)
    except openai.APIError as error:
        return RunOutcome(steps=[], failure=failure_of(error))
    return await run_async(call, spec, stream)


def run_spec(call: ChatResponseStreamCall, spec: ChatRunSpec, api_base: str) -> RunOutcome:
    if spec.mode == "sync":
        return sync_run(call, spec, api_base)
    return asyncio.run(async_run(call, spec, api_base))


def run_chat_response_stream(call: ChatResponseStreamCall, api_base: str) -> str:
    runs = [run_spec(call, spec, api_base) for spec in call.runs]
    outcome: Outcome = {"chat_response_stream": {"value": {"runs": runs}}}
    dumped = TypeAdapter(Outcome).dump_json(outcome, by_alias=True, exclude_unset=True)
    return dumped.decode()
