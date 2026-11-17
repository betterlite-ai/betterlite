import asyncio
import logging
from collections.abc import AsyncIterator, Sized
from typing import Final, Literal, NamedTuple, NoReturn, Protocol, TypedDict

import httpx
from litellm.llms.anthropic.pass_through.adapters.streaming_iterator import (
    AnthropicSSEStream,
)
from litellm.llms.anthropic.pass_through.messages.streaming_iterator import (
    aclose_if_supported,
)
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.llms.anthropic import AppliedEdit, CompactionBlock, UsageIteration
from litellm.types.utils import ModelResponseStream
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter
from pydantic import JsonValue as Json
from typing_extensions import override

from messages_sync_stream import (
    CHUNK,
    EVENT,
    AdapterErrors,
    Diagnostic,
    Initial,
    Message,
    MessagesSyncStreamCall,
    ProviderChunks,
    Wrapper,
    frames,
)

PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException


class ReportedError(BaseModel):
    status_code: int
    message: str


class EmptyChunkError(Exception):
    pass


class FailureSpec(BaseModel):
    model_config = ConfigDict(extra="forbid")

    kind: Literal["reported", "empty"]
    status_code: int = 500
    message: str = ""
    after: int

    def raise_it(self) -> NoReturn:
        if self.kind == "reported":
            raise BaseLLMException(status_code=self.status_code, message=self.message)
        raise EmptyChunkError


class ChatRunSpec(BaseModel):
    model_config = ConfigDict(extra="forbid")

    initial: Initial = Field(default_factory=Initial)
    mode: Literal["wrapper", "sse", "view"]
    failure: FailureSpec | None = None
    steps: list[Literal["next", "drain", "close"]]


class MessagesChatStreamCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    messages: list[Message]
    api_key: str
    timeout_seconds: int
    tool_name_mapping: dict[str, str] = Field(default_factory=dict)
    applied_edits: list[AppliedEdit] = Field(default_factory=list)
    compaction_block: CompactionBlock | None = None
    iterations_usage: list[UsageIteration] | None = None
    runs: list[ChatRunSpec]


class FailureOutcome(TypedDict):
    kind: str
    status_code: int | None
    message: str


class EventOutcome(NamedTuple):
    event: dict[str, Json] | None
    failure: FailureOutcome | None


class ViewOutcome(TypedDict):
    model: str
    chunks: list[str] | None


class StepOutcome(TypedDict):
    events: list[dict[str, Json]]
    frames: list[str]
    end: bool
    failure: FailureOutcome | None
    pulls: int
    diagnostics: list[Diagnostic]
    view: ViewOutcome | None


class RunOutcome(TypedDict):
    steps: list[StepOutcome]


class Outcome(TypedDict):
    messages_chat_stream: dict[Literal["value"], dict[Literal["runs"], list[RunOutcome]]]


class AsyncEventStream(Protocol):
    async def __anext__(self) -> Sized: ...


class AsyncProviderChunks(ProviderChunks):
    def __init__(self, payloads: list[bytes], failure: FailureSpec | None) -> None:
        super().__init__(payloads)
        self.failure = failure
        self.chunks: list[ModelResponseStream] = []

    @override
    async def __anext__(self) -> ModelResponseStream:
        self.pulls += 1
        if self.failure is not None and self.pulls > self.failure.after:
            self.failure.raise_it()
        if self.index >= len(self.payloads):
            raise StopAsyncIteration
        payload = self.payloads[self.index]
        self.index += 1
        # LiteLLM raises a bare Exception for a null or "None" chunk, before any translation
        if payload in (b"null", b'"None"'):
            raise EmptyChunkError
        chunk = CHUNK.validate_json(payload)
        self.chunks.append(chunk)
        return chunk


async def next_event(stream: AsyncEventStream) -> EventOutcome:
    try:
        event = await stream.__anext__()
    except StopAsyncIteration:
        return EventOutcome(None, None)
    except PROVIDER_ERROR as raised:
        record = ReportedError.model_validate(raised, from_attributes=True)
        return EventOutcome(
            None,
            FailureOutcome(kind="reported", status_code=record.status_code, message=record.message),
        )
    except EmptyChunkError as error:
        return EventOutcome(
            None,
            FailureOutcome(kind="unclassified", status_code=None, message=str(error)),
        )
    return EventOutcome(EVENT.validate_python(event), None)


def take_diagnostics(errors: AdapterErrors) -> list[Diagnostic]:
    entries = errors.entries
    errors.entries = []
    return entries


async def wrapper_step(
    step: Literal["next", "drain", "close"],
    stream: AsyncEventStream,
    source: ProviderChunks,
    errors: AdapterErrors,
) -> StepOutcome:
    events: list[dict[str, Json]] = []
    failure: FailureOutcome | None = None
    end = False
    if step != "close":
        event, failure = await next_event(stream)
        while event is not None:
            events.append(event)
            if step != "drain":
                break
            event, failure = await next_event(stream)
        end = event is None and failure is None
    return StepOutcome(
        events=events,
        frames=[],
        end=end,
        failure=failure,
        pulls=source.pulls,
        diagnostics=take_diagnostics(errors),
        view=None,
    )


async def frame_step(
    step: Literal["next", "drain", "close"],
    stream: AsyncIterator[bytes],
    source: ProviderChunks,
    errors: AdapterErrors,
    view: AnthropicSSEStream | None,
) -> StepOutcome:
    collected: list[str] = []
    frame: bytes | None = None
    if step == "close":
        if view is None:
            await aclose_if_supported(stream)
        else:
            await view.aclose()
    else:
        frame = await anext(stream, None)
        if frame is not None:
            collected.append(frame.decode())
        while step == "drain" and frame is not None:
            frame = await anext(stream, None)
            if frame is not None:
                collected.append(frame.decode())
    view_meta: ViewOutcome | None = None
    if view is not None:
        seen = view.chunks
        view_meta = ViewOutcome(
            model=view.model, chunks=None if seen is None else [chunk.id for chunk in seen]
        )
    return StepOutcome(
        events=[],
        frames=collected,
        end=step != "close" and frame is None,
        failure=None,
        pulls=source.pulls,
        diagnostics=take_diagnostics(errors),
        view=view_meta,
    )


async def run_steps(
    spec: ChatRunSpec,
    wrapper: Wrapper,
    source: AsyncProviderChunks,
    errors: AdapterErrors,
) -> list[StepOutcome]:
    if spec.mode == "wrapper":
        events: AsyncEventStream = wrapper
        return [await wrapper_step(step, events, source, errors) for step in spec.steps]
    view = AnthropicSSEStream(wrapper) if spec.mode == "view" else None
    frame_stream: AsyncIterator[bytes] = (
        view if view is not None else wrapper.async_anthropic_sse_wrapper()
    )
    return [await frame_step(step, frame_stream, source, errors, view) for step in spec.steps]


async def run_one(
    call: MessagesChatStreamCall, spec: ChatRunSpec, api_base: str, client: httpx.AsyncClient
) -> RunOutcome:
    messages: list[Json] = [
        {"role": message["role"], "content": message["content"]} for message in call.messages
    ]
    request: dict[str, Json] = {
        "model": call.model,
        "messages": messages,
        "stream": True,
        "stream_options": {"include_usage": True},
    }
    response = await client.post(f"{api_base}/chat/completions", json=request)
    source = AsyncProviderChunks(frames(response.content), spec.failure)
    settings = MessagesSyncStreamCall(
        model=call.model,
        messages=call.messages,
        api_key=call.api_key,
        timeout_seconds=call.timeout_seconds,
        tool_name_mapping=dict(call.tool_name_mapping),
        applied_edits=list(call.applied_edits),
        compaction_block=call.compaction_block,
        iterations_usage=call.iterations_usage,
        runs=[],
    )
    wrapper = Wrapper(settings, source, spec.initial)
    errors = AdapterErrors()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(errors)
    try:
        steps = await run_steps(spec, wrapper, source, errors)
    finally:
        logger.removeHandler(errors)
    return RunOutcome(steps=steps)


async def run_all(call: MessagesChatStreamCall, api_base: str) -> list[RunOutcome]:
    async with httpx.AsyncClient(trust_env=False, timeout=float(call.timeout_seconds)) as client:
        return [await run_one(call, spec, api_base, client) for spec in call.runs]


def run_messages_chat_stream(call: MessagesChatStreamCall, api_base: str) -> str:
    runs = asyncio.run(run_all(call, api_base))
    outcome: Outcome = {"messages_chat_stream": {"value": {"runs": runs}}}
    return TypeAdapter(Outcome).dump_json(outcome).decode()
