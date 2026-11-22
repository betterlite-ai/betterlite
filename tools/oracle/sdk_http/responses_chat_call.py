import asyncio
from collections.abc import Coroutine, Mapping
from typing import Final, Protocol

import openai
from litellm.proxy import proxy_server
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.responses.litellm_completion_transformation.handler import (
    LiteLLMCompletionTransformationHandler,
)
from litellm.responses.litellm_completion_transformation.streaming_iterator import (
    LiteLLMCompletionStreamingIterator as Iterator,
)
from litellm.responses.streaming_iterator import BaseResponsesAPIStreamingIterator as StreamBase
from litellm.types.llms.base import BaseLiteLLMOpenAIResponseObject as BaseResponse
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.llms.openai import ResponsesAPIOptionalRequestParams as RequestParams
from openai.types.responses.response_input_param import ResponseInputParam
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

from openai_chat import Failure, project_failure
from responses_compact_transport import INPUT, JSON, Mode
from responses_create_transport import JSON_MAP

# The handler answers directly in the sync mode and with a coroutine in the async one
Work = (
    ResponsesAPIResponse
    | StreamBase
    | Coroutine[object, object, ResponsesAPIResponse | StreamBase]
)

RESPONSE: Final[TypeAdapter[ResponsesAPIResponse]] = TypeAdapter(ResponsesAPIResponse)


class ResponseEntry(Protocol):
    def __call__(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/handler.py#L26-L95
        self,
        model: str,
        response_input: str | ResponseInputParam,
        responses_api_request: RequestParams,
        /,
        *,
        custom_llm_provider: str | None = None,
        _is_async: bool = False,
        stream: bool | None = None,
        extra_headers: Mapping[str, JsonValue] | None = None,
        **kwargs: JsonValue,
    ) -> Work: ...


class Handlers(Protocol):
    @property
    def response_api_handler(self) -> ResponseEntry: ...


# The pinned handler LiteLLM's Responses route builds a chat completion call with
HANDLER: Handlers = LiteLLMCompletionTransformationHandler()


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


class UnexpectedAnswerError(ValueError):
    def __init__(self) -> None:
        super().__init__("the handler answered outside the Responses stream or object")


class MissingEventTypeError(ValueError):
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


# The iterator names the request, its input and the metadata its logging keeps
class IteratorView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    model: str
    custom_llm_provider: str | None
    request_input: JsonValue
    responses_api_request: JsonValue
    litellm_metadata: JsonValue


# The orchestrator's iterator reads as Any to mypy, so pydantic's field check is the boundary
class HeldIterator(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)

    iterator: Iterator


def as_iterator(value: StreamBase) -> Iterator:
    fields: dict[str, StreamBase] = {"iterator": value}
    return HeldIterator.model_validate(fields).iterator


def stream_flag(request: dict[str, JsonValue]) -> bool | None:
    match request.get("stream"):
        case bool() as value:
            return value
        case None | int() | float() | str() | list() | dict():
            return None


# LiteLLM's dict.get reads, in order: the caller's kwargs, the mapped request, then the skip flag
def invoke(call: Call, api_base: str) -> Work:
    request = call.request
    arguments: dict[str, JsonValue] = {
        **call.kwargs,
        "api_base": api_base,
        "api_key": call.api_key,
        "num_retries": 0,
        "timeout": call.timeout_seconds,
    }
    return HANDLER.response_api_handler(
        call.model,
        INPUT.validate_python(request["input"]),
        request,  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/handler.py#L29
        custom_llm_provider="openai",
        _is_async=call.mode == "async",
        stream=stream_flag(request),
        extra_headers=call.extra_headers,
        **arguments,
    )


def event_type(event: BaseResponse) -> str:
    dumped = JSON_MAP.validate_json(event.model_dump_json())
    match dumped.get("type"):
        case str() as name:
            return name
        case None | bool() | int() | float() | list() | dict():
            raise MissingEventTypeError


def collect_sync(iterator: Iterator) -> list[str]:
    types: list[str] = []
    while True:
        try:
            types.append(event_type(next(iterator)))
        except StopIteration:
            return types


async def collect_async(iterator: Iterator) -> list[str]:
    types: list[str] = []
    while True:
        try:
            types.append(event_type(await iterator.__anext__()))
        except StopAsyncIteration:
            return types


def sync_events(value: ResponsesAPIResponse | StreamBase) -> list[str]:
    match value:
        case StreamBase():
            return collect_sync(as_iterator(value))
        case object():
            return []


async def async_events(value: ResponsesAPIResponse | StreamBase) -> list[str]:
    match value:
        case StreamBase():
            return await collect_async(as_iterator(value))
        case object():
            return []


def response_outcome(response: ResponsesAPIResponse, recorder: Recorder) -> Outcome:
    return Outcome(
        response=JSON.validate_json(response.model_dump_json()),
        lookups=recorder.lookups,
    )


def stream_outcome(iterator: Iterator, events: list[str], recorder: Recorder) -> Outcome:
    view = IteratorView.model_validate(iterator)
    return Outcome(
        stream=StreamView(
            model=view.model,
            custom_llm_provider=view.custom_llm_provider,
            request_input=view.request_input,
            responses_api_request=view.responses_api_request,
            litellm_metadata=view.litellm_metadata,
            event_types=events,
        ),
        lookups=recorder.lookups,
    )


def resolved(
    value: ResponsesAPIResponse | StreamBase, events: list[str], recorder: Recorder
) -> Outcome:
    match value:
        case StreamBase():
            return stream_outcome(as_iterator(value), events, recorder)
        case object():
            return response_outcome(RESPONSE.validate_python(value), recorder)


def observed_sync(work: Work, recorder: Recorder) -> Outcome:
    match work:
        case Coroutine():
            raise UnexpectedAnswerError
        case object():
            return resolved(work, sync_events(work), recorder)


async def observed_async(work: Work, recorder: Recorder) -> Outcome:
    match work:
        case Coroutine():
            value = await work
            return resolved(value, await async_events(value), recorder)
        case object():
            return resolved(work, await async_events(work), recorder)


# B011 projects the provider's error; the message text is compared apart from it
def rejected(error: openai.APIError) -> Failure:
    projected = project_failure(error)
    if projected.failure is None:
        raise UnexpectedAnswerError
    return projected.failure


def run(call: Call, api_base: str) -> Outcome:
    recorder = install_history(call.history)
    try:
        work = invoke(call, api_base)
        if call.mode == "async":
            return asyncio.run(observed_async(work, recorder))
        return observed_sync(work, recorder)
    except openai.APIError as error:
        return Outcome(failure=rejected(error), lookups=recorder.lookups)


class ResponsesChatCallRequest(StrictModel):
    responses_chat_call: Call

    def serialize(self, api_base: str) -> str:
        outcome = run(self.responses_chat_call, api_base)
        value = ChatCallValue(value=outcome)
        return ChatCallEnvelope(responses_chat_call=value).model_dump_json(by_alias=True)
