import asyncio
import json
from collections.abc import AsyncIterator, Iterator
from datetime import UTC, datetime
from typing import Literal, TypeAlias, TypedDict, cast

import httpx
import litellm
import openai
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.base_llm.responses.transformation import BaseResponsesAPIConfig
from litellm.responses.streaming_iterator import (
    CachedResponsesAPIStreamingIterator as Cached,
    MockResponsesAPIStreamingIterator as Mock,
    ResponsesAPIStreamingIterator as Async,
    SyncResponsesAPIStreamingIterator as Sync,
)
from litellm.types.llms.openai import ResponsesAPIResponse, ResponsesAPIStreamingResponse
from litellm.types.utils import CallTypes
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter
from pydantic import JsonValue as Json
from typing_extensions import override

from openai_chat import Failure, project_failure

VIEW: TypeAdapter[dict[str, Json]] = TypeAdapter(dict[str, Json])
RESPONSES: TypeAdapter[ResponsesAPIResponse] = TypeAdapter(ResponsesAPIResponse)


class ScriptedFailure(BaseModel):
    model_config = ConfigDict(extra="forbid")
    kind: Literal["read", "protocol", "timeout"]
    message: str


class RunSpec(BaseModel):
    model_config = ConfigDict(extra="forbid")
    mode: Literal["live", "scripted", "fake", "cached"]
    asynchronous: bool
    steps: list[Literal["next", "drain"]]
    payloads: list[str] = Field(default_factory=list)
    failure: ScriptedFailure | None = None
    response: Json | None = None


class ResponsesStreamDriverCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    model: str
    input: Json
    api_key: str
    timeout_seconds: int
    runs: list[RunSpec]


class StepOutcome(TypedDict):
    events: list[dict[str, Json]]
    end: bool


class RunOutcome(TypedDict):
    steps: list[StepOutcome]
    failure: Failure | None
    effects: list[str]
    logged: list[str]


Answer: TypeAlias = dict[
    Literal["responses_stream_driver"], dict[Literal["value"], list[RunOutcome]]
]


# one recorder stands in for both the LiteLLM callback and its logging object
class Recorder(CustomLogger):
    def __init__(self) -> None:
        self.effects: list[str] = []
        self.logged: list[str] = []
        self.model_call_details: dict[str, Json] = {}
        self.start_time = datetime(2026, 1, 1, tzinfo=UTC)
        self.completion_start_time: datetime | None = None

    @override
    async def async_post_call_streaming_deployment_hook(
        self,
        request_data: dict[str, Json],
        # ast-grep-ignore: no-loose-types
        response_chunk: object,
        call_type: CallTypes | None,
    ) -> None:
        del request_data, response_chunk, call_type
        self.effects.append("chunk")

    def _update_completion_start_time(self, completion_start_time: datetime) -> None:
        self.completion_start_time = completion_start_time

    async def dispatch_success_handlers(self, *args: Json, **kwargs: Json) -> None:
        del args, kwargs
        self.logged.append("success")

    async_success_handler = dispatch_success_handlers

    async def async_failure_handler(self, *args: Json, **kwargs: Json) -> None:
        del args, kwargs
        self.logged.append("failure")

    def _response_cost_calculator(self, *, result: Json) -> float | None:
        del result
        return None


def strip_nulls(value: Json) -> Json:
    match value:
        case dict():
            return {key: strip_nulls(child) for key, child in value.items() if child is not None}
        case list():
            return [strip_nulls(child) for child in value]
        case bool() | float() | int() | str() | None:
            return value


def event_view(event: ResponsesAPIStreamingResponse) -> dict[str, Json]:
    view = VIEW.validate_json(event.model_dump_json())
    return {key: strip_nulls(value) for key, value in view.items()}


def failure_from(error: openai.APIError | httpx.HTTPError) -> Failure | None:
    match error:
        case openai.APIError():
            return project_failure(error).failure
        case httpx.HTTPError():
            return Failure(cls=type(error).__name__, status=0, message=str(error))


class FailingBytes(httpx.SyncByteStream, httpx.AsyncByteStream):
    def __init__(self, chunks: list[bytes], error: Exception | None) -> None:
        self.chunks = chunks
        self.error = error

    @override
    def __iter__(self) -> Iterator[bytes]:
        yield from self.chunks
        if self.error is not None:
            raise self.error

    @override
    async def __aiter__(self) -> AsyncIterator[bytes]:
        for chunk in self.chunks:
            yield chunk
        if self.error is not None:
            raise self.error


def scripted_body(spec: RunSpec) -> FailingBytes:
    failure = spec.failure
    errors: dict[Literal["read", "protocol", "timeout"], type[httpx.HTTPError]] = {
        "read": httpx.ReadError,
        "protocol": httpx.RemoteProtocolError,
        "timeout": httpx.ReadTimeout,
    }
    error = None if failure is None else errors[failure.kind](failure.message)
    return FailingBytes([f"data: {payload}\n\n".encode() for payload in spec.payloads], error)


def recording() -> tuple[Recorder, LiteLLMLoggingObj]:
    recorder = Recorder()
    litellm.callbacks = [recorder]
    return recorder, cast("LiteLLMLoggingObj", recorder)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L556


def provider_config(model: str) -> BaseResponsesAPIConfig:
    provider = ProviderConfigManager.get_provider_responses_api_config("openai", model)
    if provider is None:
        raise ValueError("openai needs a responses provider config")
    return provider


def live_request(
    client: httpx.Client | httpx.AsyncClient, call: ResponsesStreamDriverCall, api_base: str
) -> httpx.Request:
    body: dict[str, Json] = {"model": call.model, "input": call.input, "stream": True}
    return client.build_request(
        "POST",
        f"{api_base}/responses",
        json=body,
        headers={"Authorization": f"Bearer {call.api_key}"},
    )


def sync_iterator(
    call: ResponsesStreamDriverCall, spec: RunSpec, api_base: str, stub: LiteLLMLoggingObj
) -> tuple[httpx.Client | None, Iterator[ResponsesAPIStreamingResponse]]:
    provider = provider_config(call.model)
    if spec.mode == "live":
        client = httpx.Client(timeout=float(call.timeout_seconds))
        response = client.send(live_request(client, call, api_base), stream=True)
        return client, Sync(response, call.model, provider, stub, None, "openai")
    if spec.mode == "scripted":
        body = httpx.Response(200, stream=scripted_body(spec))
        return None, Sync(body, call.model, provider, stub, None, "openai")
    if spec.mode == "fake":
        body = httpx.Response(200, content=json.dumps(spec.response))
        return None, Mock(body, call.model, provider, stub, None, "openai")
    record = RESPONSES.validate_json(json.dumps(spec.response))
    return None, Cached(record, stub)


async def async_iterator(
    call: ResponsesStreamDriverCall, spec: RunSpec, api_base: str, stub: LiteLLMLoggingObj
) -> tuple[httpx.AsyncClient | None, AsyncIterator[ResponsesAPIStreamingResponse]]:
    provider = provider_config(call.model)
    if spec.mode == "live":
        client = httpx.AsyncClient(timeout=float(call.timeout_seconds))
        response = await client.send(live_request(client, call, api_base), stream=True)
        return client, Async(response, call.model, provider, stub, None, "openai")
    if spec.mode == "scripted":
        body = httpx.Response(200, stream=scripted_body(spec))
        return None, Async(body, call.model, provider, stub, None, "openai")
    if spec.mode == "fake":
        body = httpx.Response(200, content=json.dumps(spec.response))
        return None, Mock(body, call.model, provider, stub, None, "openai")
    return None, Cached(RESPONSES.validate_json(json.dumps(spec.response)), stub)


def sync_run(
    iterator: Iterator[ResponsesAPIStreamingResponse], spec: RunSpec
) -> tuple[list[StepOutcome], Failure | None]:
    steps: list[StepOutcome] = []
    for step in spec.steps:
        events: list[dict[str, Json]] = []
        try:
            events.append(event_view(next(iterator)))
            while step == "drain":
                events.append(event_view(next(iterator)))
            steps.append(StepOutcome(events=events, end=False))
        except StopIteration:
            steps.append(StepOutcome(events=events, end=True))
        except (openai.APIError, httpx.HTTPError) as error:
            steps.append(StepOutcome(events=events, end=False))
            return steps, failure_from(error)
    return steps, None


async def async_run(
    iterator: AsyncIterator[ResponsesAPIStreamingResponse], spec: RunSpec
) -> tuple[list[StepOutcome], Failure | None]:
    steps: list[StepOutcome] = []
    for step in spec.steps:
        events: list[dict[str, Json]] = []
        failure: Failure | None = None
        end = False
        while failure is None and not end:
            try:
                events.append(event_view(await iterator.__anext__()))
            except StopAsyncIteration:
                end = True
            except (openai.APIError, httpx.HTTPError) as error:
                failure = failure_from(error)
            if step == "next" and events:
                break
        steps.append(StepOutcome(events=events, end=end))
        if failure is not None:
            return steps, failure
    return steps, None


def run_sync(call: ResponsesStreamDriverCall, spec: RunSpec, api_base: str) -> RunOutcome:
    recorder, stub = recording()
    client, iterator = sync_iterator(call, spec, api_base, stub)
    steps, failure = sync_run(iterator, spec)
    if client is not None:
        client.close()
    return RunOutcome(
        steps=steps, failure=failure, effects=recorder.effects, logged=sorted(recorder.logged)
    )


async def run_async(call: ResponsesStreamDriverCall, spec: RunSpec, api_base: str) -> RunOutcome:
    recorder, stub = recording()
    client, iterator = await async_iterator(call, spec, api_base, stub)
    steps, failure = await async_run(iterator, spec)
    if client is not None:
        await client.aclose()
    await asyncio.sleep(0)
    return RunOutcome(
        steps=steps, failure=failure, effects=recorder.effects, logged=sorted(recorder.logged)
    )


def run_spec(call: ResponsesStreamDriverCall, spec: RunSpec, api_base: str) -> RunOutcome:
    if spec.asynchronous:
        return asyncio.run(run_async(call, spec, api_base))
    return run_sync(call, spec, api_base)


class ResponsesStreamDriverRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    responses_stream_driver: ResponsesStreamDriverCall

    def serialize(self, api_base: str) -> str:
        runs = [
            run_spec(self.responses_stream_driver, spec, api_base)
            for spec in self.responses_stream_driver.runs
        ]
        outcome: Answer = {"responses_stream_driver": {"value": runs}}
        return TypeAdapter(Answer).dump_json(outcome, by_alias=True, exclude_unset=True).decode()
