import asyncio
import logging
from collections.abc import AsyncIterator, Sized
from typing import Literal, Protocol

import httpx
from litellm.llms.anthropic.pass_through.adapters.streaming_iterator import (
    AnthropicSSEStream,
)
from litellm.llms.anthropic.pass_through.messages.streaming_iterator import (
    aclose_if_supported,
)
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.utils import ModelResponseStream
from pydantic import TypeAdapter
from pydantic import JsonValue as Json
from typing_extensions import override

from messages_chat_stream_models import (
    ChatRunSpec,
    EmptyChunkError,
    EventOutcome,
    FailureOutcome,
    FailureSpec,
    MessagesChatStreamCall,
    Outcome,
    RunOutcome,
    StepOutcome,
    ViewOutcome,
)
from messages_sync_stream import (
    CHUNK,
    EVENT,
    AdapterErrors,
    Diagnostic,
    MessagesSyncStreamCall,
    ProviderChunks,
    Wrapper,
    frames,
)


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


def settings_of(call: MessagesChatStreamCall) -> MessagesSyncStreamCall:
    return MessagesSyncStreamCall(
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


async def next_event(stream: AsyncEventStream) -> EventOutcome:
    try:
        event = await stream.__anext__()
    except StopAsyncIteration:
        return EventOutcome(event=None, failure=None)
    except BaseLLMException as error:
        return EventOutcome(
            event=None,
            failure=FailureOutcome(
                kind="reported", status_code=error.status_code, message=error.message
            ),
        )
    except EmptyChunkError as error:
        return EventOutcome(
            event=None,
            failure=FailureOutcome(kind="unclassified", status_code=None, message=str(error)),
        )
    return EventOutcome(event=EVENT.validate_python(event), failure=None)


async def next_frame(stream: AsyncIterator[bytes]) -> bytes | None:
    try:
        frame = await anext(stream)
    except StopAsyncIteration:
        return None
    return frame


def take_diagnostics(errors: AdapterErrors) -> list[Diagnostic]:
    diagnostics = errors.entries
    errors.entries = []
    return diagnostics


async def wrapper_step(
    step: Literal["next", "drain", "close"],
    stream: AsyncEventStream,
    source: AsyncProviderChunks,
    errors: AdapterErrors,
) -> StepOutcome:
    events: list[dict[str, Json]] = []
    failure: FailureOutcome | None = None
    end = False
    if step != "close":
        outcome = await next_event(stream)
        event = outcome["event"]
        failure = outcome["failure"]
        while event is not None:
            events.append(event)
            if step != "drain":
                break
            outcome = await next_event(stream)
            event = outcome["event"]
            failure = outcome["failure"]
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


def observe_view(view: AnthropicSSEStream) -> ViewOutcome:
    seen = view.chunks
    chunks: list[str] | None = None
    if seen is not None:
        chunks = [chunk.id for chunk in seen]
    return ViewOutcome(model=view.model, chunks=chunks)


async def frame_step(
    step: Literal["next", "drain", "close"],
    stream: AsyncIterator[bytes],
    source: AsyncProviderChunks,
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
        frame = await next_frame(stream)
        if frame is not None:
            collected.append(frame.decode())
        while step == "drain" and frame is not None:
            frame = await next_frame(stream)
            if frame is not None:
                collected.append(frame.decode())
    return StepOutcome(
        events=[],
        frames=collected,
        end=step != "close" and frame is None,
        failure=None,
        pulls=source.pulls,
        diagnostics=take_diagnostics(errors),
        view=None if view is None else observe_view(view),
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
    wrapper = Wrapper(settings_of(call), source, spec.initial)
    errors = AdapterErrors()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(errors)
    try:
        steps = await run_steps(spec, wrapper, source, errors)
    finally:
        logger.removeHandler(errors)
    return RunOutcome(steps=steps)


async def run_all(call: MessagesChatStreamCall, api_base: str) -> list[RunOutcome]:
    runs: list[RunOutcome] = []
    async with httpx.AsyncClient(trust_env=False, timeout=float(call.timeout_seconds)) as client:
        for spec in call.runs:
            runs.append(await run_one(call, spec, api_base, client))
    return runs


def run_messages_chat_stream(call: MessagesChatStreamCall, api_base: str) -> str:
    runs = asyncio.run(run_all(call, api_base))
    outcome: Outcome = {"messages_chat_stream": {"value": {"runs": runs}}}
    return TypeAdapter(Outcome).dump_json(outcome).decode()
