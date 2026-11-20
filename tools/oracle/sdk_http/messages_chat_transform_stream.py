import asyncio
import re
from collections.abc import AsyncIterator, Iterator
from typing import Final, Literal, Protocol, TypedDict, runtime_checkable

import httpx
from litellm.llms.anthropic.pass_through.adapters.streaming_iterator import (
    AnthropicSSEStream,
)
from litellm.llms.anthropic.pass_through.adapters.transformation import AnthropicAdapter
from litellm.llms.anthropic.pass_through.context_management.result import PolyfillResult
from litellm.types.llms.anthropic import AppliedEdit, CompactionBlock, UsageIteration
from litellm.types.utils import ModelResponseStream
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

from messages_sync_stream import (
    CHUNK,
    NORMALIZED_ID,
    Message,
    ProviderChunks,
    frames,
)

ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)
NO_MESSAGES: Final[list[dict[str, JsonValue]]] = []
MESSAGE_ID: Final = re.compile(
    r"msg_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"
)
Step = Literal["next", "drain", "close"]


@runtime_checkable
class StreamView(Protocol):
    # LiteLLM types messages with Any, so the view names only what a case reads
    @property
    def model(self) -> str: ...

    @property
    def messages(self) -> JsonValue: ...

    @property
    def chunks(self) -> list[ModelResponseStream] | None: ...


VIEW: Final = TypeAdapter[StreamView](StreamView, config=ARBITRARY)


class RunSpec(BaseModel):
    model_config = ConfigDict(extra="forbid")

    is_async: bool = True
    steps: list[Step]


class MessagesChatTransformCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    messages: list[Message]
    api_key: str
    timeout_seconds: int
    tool_name_mapping: dict[str, str] = Field(default_factory=dict)
    applied_edits: list[AppliedEdit] = Field(default_factory=list)
    compaction_block: CompactionBlock | None = None
    iterations_usage: list[UsageIteration] | None = None
    runs: list[RunSpec]


class ViewOutcome(TypedDict):
    model: str
    messages: JsonValue
    chunks: list[str] | None


class StepOutcome(TypedDict):
    frames: list[str]
    end: bool
    view: ViewOutcome | None


class RunOutcome(TypedDict):
    is_async: bool
    steps: list[StepOutcome]


class Outcome(TypedDict):
    messages_chat_transform_stream: dict[
        Literal["value"], dict[Literal["runs"], list[RunOutcome]]
    ]


class ProviderStream(ProviderChunks):
    # the wrapper reports its source's collected chunks and messages for disconnect-time billing
    def __init__(self, payloads: list[bytes], messages: list[JsonValue]) -> None:
        super().__init__(payloads)
        self.chunks: list[ModelResponseStream] = []
        self.messages = messages

    def pull(self) -> ModelResponseStream:
        if self.index >= len(self.payloads):
            raise StopIteration
        payload = self.payloads[self.index]
        self.index += 1
        chunk = CHUNK.validate_json(payload)
        self.chunks.append(chunk)
        return chunk

    @override
    def __next__(self) -> ModelResponseStream:
        self.pulls += 1
        return self.pull()

    @override
    async def __anext__(self) -> ModelResponseStream:
        self.pulls += 1
        try:
            return self.pull()
        except StopIteration:
            raise StopAsyncIteration from None


class SyncFrames(AsyncIterator[bytes]):
    # the sync factory mode returns a plain Iterator, so the step loop drives it as an async one
    def __init__(self, stream: Iterator[bytes]) -> None:
        self.stream = stream

    @override
    async def __anext__(self) -> bytes:
        try:
            return next(self.stream)
        except StopIteration:
            raise StopAsyncIteration from None

    async def aclose(self) -> None:
        raise ValueError("a sync run cannot be closed")


def view_outcome(view: StreamView) -> ViewOutcome:
    chunks = view.chunks
    return ViewOutcome(
        model=view.model,
        messages=view.messages,
        chunks=None if chunks is None else [chunk.id for chunk in chunks],
    )


async def steps(
    stream: AnthropicSSEStream | SyncFrames,
    view: StreamView | None,
    specs: list[Step],
) -> list[StepOutcome]:
    outcomes: list[StepOutcome] = []
    for spec in specs:
        written: list[str] = []
        end = False
        match spec:
            case "close":
                await stream.aclose()
            case "next":
                frame = await anext(stream, None)
                if frame is None:
                    end = True
                else:
                    written.append(MESSAGE_ID.sub(NORMALIZED_ID, frame.decode()))
            case "drain":
                frame = await anext(stream, None)
                while frame is not None:
                    written.append(MESSAGE_ID.sub(NORMALIZED_ID, frame.decode()))
                    frame = await anext(stream, None)
                end = True
        outcomes.append(
            StepOutcome(
                frames=written,
                end=end,
                view=None if view is None else view_outcome(view),
            )
        )
    return outcomes


def polyfill_result(call: MessagesChatTransformCall) -> PolyfillResult | None:
    if (
        not call.applied_edits
        and call.compaction_block is None
        and call.iterations_usage is None
    ):
        return None
    return PolyfillResult(
        messages=NO_MESSAGES,
        system=None,
        applied_edits=list(call.applied_edits),
        compaction_block=call.compaction_block,
        iterations_usage=call.iterations_usage,
    )


async def run_one(
    call: MessagesChatTransformCall, spec: RunSpec, api_base: str, client: httpx.AsyncClient
) -> RunOutcome:
    messages: list[JsonValue] = [
        {"role": message["role"], "content": message["content"]} for message in call.messages
    ]
    request: dict[str, JsonValue] = {
        "model": call.model,
        "messages": messages,
        "stream": True,
        "stream_options": {"include_usage": True},
    }
    response = await client.post(f"{api_base}/chat/completions", json=request)
    source = ProviderStream(frames(response.content), messages)
    result = AnthropicAdapter().translate_completion_output_params_streaming(
        completion_stream=source,
        model=call.model,
        tool_name_mapping=dict(call.tool_name_mapping),
        polyfill_result=polyfill_result(call),
        is_async=spec.is_async,
    )
    if result is None:
        raise ValueError("the streaming factory returned no iterator")
    match result:
        case AnthropicSSEStream():
            view = VIEW.validate_python(result)
            return RunOutcome(is_async=True, steps=await steps(result, view, spec.steps))
        case Iterator():
            sync = SyncFrames(result)
            return RunOutcome(is_async=False, steps=await steps(sync, None, spec.steps))
        # the factory returns the view or the iterator above, so any other async iterator is a bug
        case object():
            raise TypeError("the streaming factory returned an unknown iterator")


async def run_all(
    call: MessagesChatTransformCall, api_base: str
) -> list[RunOutcome]:
    async with httpx.AsyncClient(trust_env=False, timeout=float(call.timeout_seconds)) as client:
        return [await run_one(call, spec, api_base, client) for spec in call.runs]


def run_transform_stream(
    call: MessagesChatTransformCall, api_base: str
) -> str:
    runs = asyncio.run(run_all(call, api_base))
    outcome: Outcome = {"messages_chat_transform_stream": {"value": {"runs": runs}}}
    return TypeAdapter(Outcome).dump_json(outcome).decode()


class ChatTransformStreamRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages_chat_transform_stream: MessagesChatTransformCall

    def serialize(self, api_base: str) -> str:
        return run_transform_stream(self.messages_chat_transform_stream, api_base)
