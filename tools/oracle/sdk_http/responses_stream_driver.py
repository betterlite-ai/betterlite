import asyncio
import json
from collections.abc import AsyncIterator, Iterator
from datetime import UTC, datetime
from typing import Literal, cast

import httpx
import litellm
import openai
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.openai.responses.transformation import OpenAIResponsesAPIConfig
from litellm.responses.streaming_iterator import (
    CachedResponsesAPIStreamingIterator,
    MockResponsesAPIStreamingIterator,
    ResponsesAPIStreamingIterator,
    ResponsesAPIStreamingResponse,
    SyncResponsesAPIStreamingIterator,
)
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.utils import CallTypes
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

    mode: Literal[
        "live_sync",
        "live_async",
        "scripted_sync",
        "scripted_async",
        "fake_sync",
        "fake_async",
        "cached_sync",
        "cached_async",
    ]
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


class StepOutcome(BaseModel):
    events: list[dict[str, Json]]
    end: bool


class RunOutcome(BaseModel):
    steps: list[StepOutcome]
    failure: Failure | None
    effects: list[str]
    logged: list[str]


class DriverValue(BaseModel):
    value: list[RunOutcome]


class Outcome(BaseModel):
    responses_stream_driver: DriverValue


class Recorder(CustomLogger):
    def __init__(self) -> None:
        self.effects: list[str] = []

    @override
    async def async_post_call_streaming_deployment_hook(
        self,
        request_data: dict[str, Json],
        response_chunk: object,
        call_type: CallTypes | None,
    ) -> object | None:
        del request_data, call_type
        self.effects.append("chunk")
        return response_chunk


class RecordingLogging:
    def __init__(self) -> None:
        self.model_call_details: dict[str, Json] = {}
        self.effects: list[str] = []
        self.start_time = datetime(2026, 1, 1, tzinfo=UTC)
        self.completion_start_time: datetime | None = None

    def post_call(self, original_response: str | None = None, *args: Json, **kwargs: Json) -> None:
        del original_response, args, kwargs

    def _update_completion_start_time(self, completion_start_time: datetime) -> None:
        self.completion_start_time = completion_start_time

    async def dispatch_success_handlers(self, *args: Json, **kwargs: Json) -> None:
        del args, kwargs
        self.effects.append("success")

    async def async_success_handler(self, *args: Json, **kwargs: Json) -> None:
        del args, kwargs
        self.effects.append("success")

    def success_handler(self, *args: Json, **kwargs: Json) -> None:
        del args, kwargs

    async def async_failure_handler(self, *args: Json, **kwargs: Json) -> None:
        del args, kwargs
        self.effects.append("failure")

    def failure_handler(self, *args: Json, **kwargs: Json) -> None:
        del args, kwargs

    def _response_cost_calculator(self, *, result: Json) -> float | None:
        del result
        return None


def logging_object() -> tuple[LiteLLMLoggingObj, RecordingLogging]:
    stub = RecordingLogging()
    typed = cast("LiteLLMLoggingObj", stub)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L556
    return typed, stub


def strip_nulls(value: Json) -> Json:
    match value:
        case dict():
            return {
                key: strip_nulls(child)
                for key, child in value.items()
                if child is not None
            }
        case list():
            return [strip_nulls(child) for child in value]
        case bool() | float() | int() | str() | None:
            return value


def event_view(event: ResponsesAPIStreamingResponse) -> dict[str, Json]:
    view = VIEW.validate_json(event.model_dump_json())
    return {key: strip_nulls(value) for key, value in view.items()}


def failure_from(error: openai.APIError | httpx.HTTPError) -> Failure:
    match error:
        case openai.APIError():
            outcome = project_failure(error)
            failure = outcome.failure
            if failure is None:
                raise ValueError("the mapped error carried no failure")
            return failure
        case httpx.HTTPError():
            return Failure(cls=type(error).__name__, status=0, message=str(error))


def scripted_error(failure: ScriptedFailure) -> Exception:
    match failure.kind:
        case "read":
            return httpx.ReadError(failure.message)
        case "protocol":
            return httpx.RemoteProtocolError(failure.message)
        case "timeout":
            return httpx.ReadTimeout(failure.message)


class FailingBytes(httpx.SyncByteStream):
    def __init__(self, chunks: list[bytes], error: Exception | None) -> None:
        self.chunks = chunks
        self.error = error

    def __iter__(self) -> Iterator[bytes]:
        return self

    def __next__(self) -> bytes:
        if self.chunks:
            return self.chunks.pop(0)
        if self.error is None:
            raise StopIteration
        raise self.error


class AsyncFailingBytes(httpx.AsyncByteStream):
    def __init__(self, chunks: list[bytes], error: Exception | None) -> None:
        self.chunks = chunks
        self.error = error

    def __aiter__(self) -> AsyncIterator[bytes]:
        return self

    async def __anext__(self) -> bytes:
        if self.chunks:
            return self.chunks.pop(0)
        if self.error is None:
            raise StopAsyncIteration
        raise self.error


def frames(payloads: list[str]) -> list[bytes]:
    return [f"data: {payload}\n\n".encode() for payload in payloads]


def scripted_error_of(spec: RunSpec) -> Exception | None:
    failure = spec.failure
    if failure is None:
        return None
    return scripted_error(failure)


def outcome(
    steps: list[StepOutcome],
    failure: Failure | None,
    recorder: Recorder,
    stub: RecordingLogging,
) -> RunOutcome:
    return RunOutcome(
        steps=steps,
        failure=failure,
        effects=recorder.effects,
        logged=sorted(stub.effects),
    )


def drive_sync(
    iterator: SyncResponsesAPIStreamingIterator, spec: RunSpec
) -> tuple[list[StepOutcome], Failure | None]:
    steps: list[StepOutcome] = []
    for step in spec.steps:
        events: list[dict[str, Json]] = []
        end = False
        failure: Failure | None = None
        while True:
            try:
                event = next(iterator)
            except StopIteration:
                end = True
                break
            except (openai.APIError, httpx.HTTPError) as error:
                failure = failure_from(error)
                break
            events.append(event_view(event))
            if step == "next":
                break
        steps.append(StepOutcome(events=events, end=end))
        if failure is not None:
            return steps, failure
    return steps, None


async def drive_async(
    iterator: ResponsesAPIStreamingIterator, spec: RunSpec
) -> tuple[list[StepOutcome], Failure | None]:
    steps: list[StepOutcome] = []
    for step in spec.steps:
        events: list[dict[str, Json]] = []
        end = False
        failure: Failure | None = None
        while True:
            try:
                event = await iterator.__anext__()
            except StopAsyncIteration:
                end = True
                break
            except (openai.APIError, httpx.HTTPError) as error:
                failure = failure_from(error)
                break
            events.append(event_view(event))
            if step == "next":
                break
        steps.append(StepOutcome(events=events, end=end))
        if failure is not None:
            return steps, failure
    return steps, None


def request_body(call: ResponsesStreamDriverCall) -> dict[str, Json]:
    return {"model": call.model, "input": call.input, "stream": True}


def run_live_sync(
    call: ResponsesStreamDriverCall, spec: RunSpec, api_base: str
) -> RunOutcome:
    recorder = Recorder()
    litellm.callbacks = [recorder]
    typed, stub = logging_object()
    with httpx.Client(timeout=float(call.timeout_seconds)) as client:
        request = client.build_request(
            "POST",
            f"{api_base}/responses",
            json=request_body(call),
            headers={"Authorization": f"Bearer {call.api_key}"},
        )
        response = client.send(request, stream=True)
        iterator = SyncResponsesAPIStreamingIterator(
            response=response,
            model=call.model,
            responses_api_provider_config=OpenAIResponsesAPIConfig(),
            logging_obj=typed,
            custom_llm_provider="openai",
        )
        steps, failure = drive_sync(iterator, spec)
        response.close()
    return outcome(steps, failure, recorder, stub)


async def run_live_async(
    call: ResponsesStreamDriverCall, spec: RunSpec, api_base: str
) -> RunOutcome:
    recorder = Recorder()
    litellm.callbacks = [recorder]
    typed, stub = logging_object()
    async with httpx.AsyncClient(timeout=float(call.timeout_seconds)) as client:
        request = client.build_request(
            "POST",
            f"{api_base}/responses",
            json=request_body(call),
            headers={"Authorization": f"Bearer {call.api_key}"},
        )
        response = await client.send(request, stream=True)
        iterator = ResponsesAPIStreamingIterator(
            response=response,
            model=call.model,
            responses_api_provider_config=OpenAIResponsesAPIConfig(),
            logging_obj=typed,
            custom_llm_provider="openai",
        )
        steps, failure = await drive_async(iterator, spec)
        await response.aclose()
    await asyncio.sleep(0)
    return outcome(steps, failure, recorder, stub)


def run_scripted_sync(call: ResponsesStreamDriverCall, spec: RunSpec) -> RunOutcome:
    recorder = Recorder()
    litellm.callbacks = [recorder]
    typed, stub = logging_object()
    body = FailingBytes(frames(spec.payloads), scripted_error_of(spec))
    iterator = SyncResponsesAPIStreamingIterator(
        response=httpx.Response(200, stream=body),
        model=call.model,
        responses_api_provider_config=OpenAIResponsesAPIConfig(),
        logging_obj=typed,
        custom_llm_provider="openai",
    )
    steps, failure = drive_sync(iterator, spec)
    return outcome(steps, failure, recorder, stub)


async def run_scripted_async(
    call: ResponsesStreamDriverCall, spec: RunSpec
) -> RunOutcome:
    recorder = Recorder()
    litellm.callbacks = [recorder]
    typed, stub = logging_object()
    body = AsyncFailingBytes(frames(spec.payloads), scripted_error_of(spec))
    iterator = ResponsesAPIStreamingIterator(
        response=httpx.Response(200, stream=body),
        model=call.model,
        responses_api_provider_config=OpenAIResponsesAPIConfig(),
        logging_obj=typed,
        custom_llm_provider="openai",
    )
    steps, failure = await drive_async(iterator, spec)
    await asyncio.sleep(0)
    return outcome(steps, failure, recorder, stub)


def fake_sync(call: ResponsesStreamDriverCall, spec: RunSpec) -> RunOutcome:
    recorder = Recorder()
    typed, stub = logging_object()
    response = spec.response
    if response is None:
        raise ValueError("a fake run needs a response")
    iterator = MockResponsesAPIStreamingIterator(
        response=httpx.Response(200, content=json.dumps(response)),
        model=call.model,
        responses_api_provider_config=OpenAIResponsesAPIConfig(),
        logging_obj=typed,
        custom_llm_provider="openai",
    )
    steps, failure = drive_sync(iterator, spec)
    return outcome(steps, failure, recorder, stub)


async def fake_async(call: ResponsesStreamDriverCall, spec: RunSpec) -> RunOutcome:
    recorder = Recorder()
    typed, stub = logging_object()
    response = spec.response
    if response is None:
        raise ValueError("a fake run needs a response")
    iterator = MockResponsesAPIStreamingIterator(
        response=httpx.Response(200, content=json.dumps(response)),
        model=call.model,
        responses_api_provider_config=OpenAIResponsesAPIConfig(),
        logging_obj=typed,
        custom_llm_provider="openai",
    )
    steps, failure = await drive_async(iterator, spec)
    await asyncio.sleep(0)
    return outcome(steps, failure, recorder, stub)


def cached_sync(spec: RunSpec) -> RunOutcome:
    recorder = Recorder()
    typed, stub = logging_object()
    response = spec.response
    if response is None:
        raise ValueError("a cached run needs a response")
    iterator = CachedResponsesAPIStreamingIterator(
        response=RESPONSES.validate_json(json.dumps(response)),
        logging_obj=typed,
    )
    steps, failure = drive_sync(iterator, spec)
    return outcome(steps, failure, recorder, stub)


async def cached_async(spec: RunSpec) -> RunOutcome:
    recorder = Recorder()
    typed, stub = logging_object()
    response = spec.response
    if response is None:
        raise ValueError("a cached run needs a response")
    iterator = CachedResponsesAPIStreamingIterator(
        response=RESPONSES.validate_json(json.dumps(response)),
        logging_obj=typed,
    )
    steps, failure = await drive_async(iterator, spec)
    await asyncio.sleep(0)
    return outcome(steps, failure, recorder, stub)


def run_spec(
    call: ResponsesStreamDriverCall, spec: RunSpec, api_base: str
) -> RunOutcome:
    if spec.mode == "live_sync":
        return run_live_sync(call, spec, api_base)
    if spec.mode == "live_async":
        return asyncio.run(run_live_async(call, spec, api_base))
    if spec.mode == "scripted_sync":
        return run_scripted_sync(call, spec)
    if spec.mode == "scripted_async":
        return asyncio.run(run_scripted_async(call, spec))
    if spec.mode == "fake_sync":
        return fake_sync(call, spec)
    if spec.mode == "fake_async":
        return asyncio.run(fake_async(call, spec))
    if spec.mode == "cached_sync":
        return cached_sync(spec)
    return asyncio.run(cached_async(spec))


def run_responses_stream_driver(
    call: ResponsesStreamDriverCall, api_base: str
) -> str:
    runs = [run_spec(call, spec, api_base) for spec in call.runs]
    outcome_value = Outcome(responses_stream_driver=DriverValue(value=runs))
    return TypeAdapter(Outcome).dump_json(
        outcome_value, by_alias=True, exclude_unset=True
    ).decode()


class ResponsesStreamDriverRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    responses_stream_driver: ResponsesStreamDriverCall

    def serialize(self, api_base: str) -> str:
        return run_responses_stream_driver(self.responses_stream_driver, api_base)
