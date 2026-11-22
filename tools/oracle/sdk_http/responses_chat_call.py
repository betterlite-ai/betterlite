import asyncio
from collections.abc import Callable, Coroutine
from typing import Final, Protocol

import openai
from litellm.proxy import proxy_server
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.responses.litellm_completion_transformation.handler import (
    LiteLLMCompletionTransformationHandler,
)
from litellm.responses.litellm_completion_transformation.streaming_iterator import (
    LiteLLMCompletionStreamingIterator as CompletionIterator,
)
from litellm.responses.streaming_iterator import BaseResponsesAPIStreamingIterator
from litellm.types.llms.openai import (
    BaseLiteLLMOpenAIResponseObject,
    ResponsesAPIResponse,
)
from pydantic import BaseModel, ConfigDict, Field, JsonValue

from openai_chat import Failure, project_failure
from responses_compact_transport import JSON, Mode
from responses_create_transport import JSON_MAP

# The handler answers directly in the sync mode and with a coroutine in the async one
ResponsesWork = (
    ResponsesAPIResponse
    | BaseResponsesAPIStreamingIterator
    | Coroutine[object, object, ResponsesAPIResponse | BaseResponsesAPIStreamingIterator]
)

ResponseEntry = Callable[..., ResponsesWork]


class Handlers(Protocol):
    @property
    def response_api_handler(self) -> ResponseEntry: ...


# The pinned handler LiteLLM's Responses route builds a chat completion call with
HANDLER: Final[Handlers] = LiteLLMCompletionTransformationHandler()


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid")


class Call(StrictModel):
    mode: Mode
    model: str
    request: dict[str, JsonValue]
    api_key: str
    timeout_seconds: int
    extra_headers: dict[str, JsonValue] | None = None
    kwargs: dict[str, JsonValue] = Field(default_factory=dict)
    history: list[dict[str, JsonValue]] = Field(default_factory=list)


class StreamView(StrictModel):
    model: str
    custom_llm_provider: str | None
    request_input: JsonValue
    responses_api_request: JsonValue
    litellm_metadata: JsonValue
    event_types: list[str]


class Outcome(StrictModel):
    response: JsonValue = None
    stream: StreamView | None = None
    failure: Failure | None = None
    lookups: list[str] = Field(default_factory=list)


class ChatCallValue(StrictModel):
    value: Outcome


class ChatCallEnvelope(StrictModel):
    responses_chat_call: ChatCallValue


class UnexpectedAnswer(ValueError):
    def __init__(self) -> None:
        super().__init__("the handler answered outside the Responses stream or object")


class MissingEventType(ValueError):
    def __init__(self) -> None:
        super().__init__("a Responses event carries no type")


# The rows the case scripts stand in for LiteLLM_SpendLogs; every lookup is kept for comparison
class Recorder:
    def __init__(self, rows: list[dict[str, JsonValue]]) -> None:
        self.rows = rows
        self.lookups: list[str] = []


class FakeWrapper(PrismaWrapper):
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder

    async def query_raw(self, _sql: str, request_id: str) -> list[dict[str, JsonValue]]:
        self.recorder.lookups.append(request_id)
        return self.recorder.rows


class FakePrismaClient(PrismaClient):
    def __init__(self, wrapper: FakeWrapper) -> None:
        self.db = wrapper


# the session handler resolves both names at call time, so the module attribute is the seam
def install_history(rows: list[dict[str, JsonValue]]) -> Recorder:
    recorder = Recorder(rows)
    proxy_server.prisma_client = FakePrismaClient(FakeWrapper(recorder))
    proxy_server.disable_spend_logs = True
    return recorder


# LiteLLM's dict.get reads, in order: the caller's kwargs, the mapped request, then the skip flag
def invoke(call: Call, api_base: str) -> ResponsesWork:
    request = call.request
    arguments: dict[str, JsonValue] = {
        **call.kwargs,
        "api_base": api_base,
        "api_key": call.api_key,
        "num_retries": 0,
        "timeout": call.timeout_seconds,
    }
    # the raw list keeps the additional_tools items hoisting needs; the typed adapter strips them
    return HANDLER.response_api_handler(
        call.model,
        request["input"],
        request,
        "openai",
        call.mode == "async",
        request.get("stream"),
        call.extra_headers,
        **arguments,
    )


def event_type(event: BaseLiteLLMOpenAIResponseObject) -> str:
    dumped = JSON_MAP.validate_json(event.model_dump_json())
    name = dumped.get("type")
    if isinstance(name, str):
        return name
    raise MissingEventType


def sync_events(iterator: CompletionIterator) -> list[str]:
    types: list[str] = []
    while True:
        try:
            event = next(iterator)
        except StopIteration:
            return types
        types.append(event_type(event))


async def async_events(iterator: CompletionIterator) -> list[str]:
    types: list[str] = []
    while True:
        try:
            event = await iterator.__anext__()
        except StopAsyncIteration:
            return types
        types.append(event_type(event))


# The iterator names the request and input the events are built from, plus the id its logging keeps
def stream_view(iterator: CompletionIterator, event_types: list[str]) -> StreamView:
    return StreamView(
        model=iterator.model,
        custom_llm_provider=iterator.custom_llm_provider,
        request_input=JSON.validate_python(iterator.request_input),
        responses_api_request=JSON.validate_python(iterator.responses_api_request),
        litellm_metadata=JSON.validate_python(iterator.litellm_metadata),
        event_types=event_types,
    )


# B011 projects the provider's error; the message text is compared apart from it
def rejected(error: openai.APIError) -> Failure:
    projected = project_failure(error)
    if projected.failure is None:
        raise UnexpectedAnswer
    return projected.failure


def observe(work: ResponsesWork, recorder: Recorder) -> Outcome:
    if isinstance(work, Coroutine):
        raise ValueError("the sync handler returned a coroutine")
    match work:
        case ResponsesAPIResponse():
            return Outcome(
                response=JSON.validate_json(work.model_dump_json()),
                lookups=recorder.lookups,
            )
        case CompletionIterator():
            return Outcome(
                stream=stream_view(work, sync_events(work)),
                lookups=recorder.lookups,
            )
        case object():
            raise UnexpectedAnswer


async def observe_async(work: ResponsesWork, recorder: Recorder) -> Outcome:
    if not isinstance(work, Coroutine):
        raise ValueError("the async handler returned no coroutine")
    done = await work
    match done:
        case ResponsesAPIResponse():
            return Outcome(
                response=JSON.validate_json(done.model_dump_json()),
                lookups=recorder.lookups,
            )
        case CompletionIterator():
            return Outcome(
                stream=stream_view(done, await async_events(done)),
                lookups=recorder.lookups,
            )
        case object():
            raise UnexpectedAnswer


def run(call: Call, api_base: str) -> Outcome:
    recorder = install_history(call.history)
    try:
        work = invoke(call, api_base)
        if call.mode == "async":
            return asyncio.run(observe_async(work, recorder))
        return observe(work, recorder)
    except openai.APIError as error:
        return Outcome(failure=rejected(error), lookups=recorder.lookups)


class ResponsesChatCallRequest(StrictModel):
    responses_chat_call: Call

    def serialize(self, api_base: str) -> str:
        outcome = run(self.responses_chat_call, api_base)
        value = ChatCallValue(value=outcome)
        return ChatCallEnvelope(responses_chat_call=value).model_dump_json(by_alias=True)
