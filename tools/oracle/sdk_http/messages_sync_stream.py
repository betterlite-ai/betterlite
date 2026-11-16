import logging
from collections.abc import AsyncIterator, Iterator
from typing import Final, Literal, Protocol

import httpx
from litellm.llms.anthropic.pass_through.adapters.streaming_iterator import (
    AnthropicStreamWrapper,
)
from litellm.types.llms.anthropic import (
    AppliedEdit,
    CompactionBlock,
    MessageBlockDelta,
    MessageDelta,
    UsageDelta,
    UsageIteration,
)
from litellm.types.utils import ModelResponseStream
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter
from pydantic import JsonValue as Json
from typing_extensions import override

from messages_sync_stream_outcome import (
    Diagnostic,
    Outcome,
    RunOutcome,
    State,
    StepOutcome,
    SyncOutcome,
    SyncValue,
)

ADAPTER_PREFIX = "Anthropic Adapter - "
NORMALIZED_ID = "msg_00000000-0000-0000-0000-000000000000"
PAYLOAD = TypeAdapter(dict[str, Json])
CHUNK: Final[TypeAdapter[ModelResponseStream]] = TypeAdapter(ModelResponseStream)


class Message(BaseModel):
    model_config = ConfigDict(extra="forbid")

    role: str
    content: str


class TextDelta(BaseModel):
    model_config = ConfigDict(extra="forbid")

    type: Literal["text_delta"]
    text: str


class HeldDelta(BaseModel):
    model_config = ConfigDict(extra="forbid")

    type: Literal["content_block_delta"]
    index: int
    delta: TextDelta


class HeldStop(BaseModel):
    model_config = ConfigDict(extra="forbid")

    type: Literal["message_delta"]
    delta: MessageDelta
    usage: UsageDelta | None = None


class Initial(BaseModel):
    model_config = ConfigDict(extra="forbid")

    holding_chunk: HeldDelta | None = None
    holding_stop_reason_chunk: HeldStop | None = None
    queued_usage_chunk: bool = False


class RunSpec(BaseModel):
    model_config = ConfigDict(extra="forbid")

    initial: Initial = Field(default_factory=Initial)
    steps: list[Literal["next", "drain"]]


class MessagesSyncStreamCall(BaseModel):
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


class EventStream(Protocol):
    def __next__(self) -> dict[str, Json]: ...


class ProviderChunks(Iterator[ModelResponseStream]):
    def __init__(self, payloads: list[bytes]) -> None:
        self.payloads = payloads
        self.index = 0
        self.pulls = 0

    @override
    def __iter__(self) -> "ProviderChunks":
        return self

    def __aiter__(self) -> AsyncIterator[ModelResponseStream]:
        return self

    async def __anext__(self) -> ModelResponseStream:
        raise StopAsyncIteration

    @override
    def __next__(self) -> ModelResponseStream:
        self.pulls += 1
        if self.index >= len(self.payloads):
            raise StopIteration
        payload = self.payloads[self.index]
        self.index += 1
        return CHUNK.validate_json(payload)


class Wrapper(AnthropicStreamWrapper):
    def __init__(
        self,
        call: MessagesSyncStreamCall,
        source: ProviderChunks,
        initial: Initial,
    ) -> None:
        super().__init__(
            completion_stream=source,
            model=call.model,
            tool_name_mapping=dict(call.tool_name_mapping),
            applied_edits=list(call.applied_edits),
            compaction_block=call.compaction_block,
            iterations_usage=call.iterations_usage,
        )
        self.queued_usage_chunk = initial.queued_usage_chunk
        self._message_id = NORMALIZED_ID
        held = initial.holding_chunk
        if held is not None:
            held_value: dict[str, Json] = {
                "type": "content_block_delta",
                "index": held.index,
                "delta": {"type": held.delta.type, "text": held.delta.text},
            }
            object.__setattr__(self, "holding_chunk", held_value)
        stopped = initial.holding_stop_reason_chunk
        if stopped is None:
            return
        if stopped.usage is None:
            self.holding_stop_reason_chunk = MessageBlockDelta(
                type="message_delta",
                delta=stopped.delta,
            )
            return
        self.holding_stop_reason_chunk = MessageBlockDelta(
            type="message_delta",
            delta=stopped.delta,
            usage=stopped.usage,
        )


class AdapterErrors(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.ERROR)
        self.entries: list[Diagnostic] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if not message.startswith(ADAPTER_PREFIX):
            return
        self.entries.append(Diagnostic(level=record.levelname.lower(), prefix=ADAPTER_PREFIX))

    def taken(self) -> list[Diagnostic]:
        entries, self.entries = self.entries, []
        return entries


def next_event(stream: EventStream) -> dict[str, Json] | None:
    try:
        event = next(stream)
    except StopIteration:
        return None
    return event


def observe(wrapper: Wrapper, source: ProviderChunks) -> State:
    return State(
        pulls=source.pulls,
        index=wrapper.current_content_block_index,
        block_type=wrapper.current_content_block_type,
        sent_first_chunk=wrapper.sent_first_chunk,
        sent_content_block_start=wrapper.sent_content_block_start,
        sent_content_block_finish=wrapper.sent_content_block_finish,
        sent_last_message=wrapper.sent_last_message,
        queued_usage_chunk=wrapper.queued_usage_chunk,
        holding_stop_reason=wrapper.holding_stop_reason_chunk is not None,
        sent_compaction_block=wrapper.sent_compaction_block,
        sent_compaction_block_start=wrapper.sent_compaction_block_start,
        sent_compaction_block_delta=wrapper.sent_compaction_block_delta,
    )


def run_steps(
    spec: RunSpec,
    wrapper: Wrapper,
    source: ProviderChunks,
    errors: AdapterErrors,
) -> list[StepOutcome]:
    steps: list[StepOutcome] = []
    for step in spec.steps:
        events: list[dict[str, Json]] = []
        if step == "next":
            event = next_event(wrapper)
            if event is not None:
                events.append(event)
            steps.append(
                StepOutcome(
                    events=events,
                    end=event is None,
                    state=observe(wrapper, source),
                    diagnostics=errors.taken(),
                )
            )
            continue
        event = next_event(wrapper)
        while event is not None:
            events.append(event)
            event = next_event(wrapper)
        steps.append(
            StepOutcome(
                events=events,
                end=True,
                state=observe(wrapper, source),
                diagnostics=errors.taken(),
            )
        )
    return steps


def body(call: MessagesSyncStreamCall) -> dict[str, Json]:
    messages: list[Json] = [
        {"role": message.role, "content": message.content} for message in call.messages
    ]
    return {
        "model": call.model,
        "messages": messages,
        "stream": True,
        "stream_options": {"include_usage": True},
    }


def frames(content: bytes) -> list[bytes]:
    payloads: list[bytes] = []
    for part in content.split(b"\n\n"):
        lines = [
            line.removeprefix(b"data:").removeprefix(b" ")
            for line in part.split(b"\n")
            if line.startswith(b"data:")
        ]
        payload = b"\n".join(lines)
        if payload:
            payloads.append(payload)
    return payloads


def run_one(call: MessagesSyncStreamCall, spec: RunSpec, api_base: str) -> RunOutcome:
    with httpx.Client(trust_env=False, timeout=float(call.timeout_seconds)) as client:
        response = client.post(f"{api_base}/chat/completions", json=body(call))
    source = ProviderChunks(frames(response.content))
    wrapper = Wrapper(call, source, spec.initial)
    errors = AdapterErrors()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(errors)
    try:
        steps = run_steps(spec, wrapper, source, errors)
    finally:
        logger.removeHandler(errors)
    return RunOutcome(steps=steps)


def run_messages_sync_stream(call: MessagesSyncStreamCall, api_base: str) -> str:
    runs = [run_one(call, spec, api_base) for spec in call.runs]
    outcome = Outcome(messages_sync_stream=SyncOutcome(value=SyncValue(runs=runs)))
    return outcome.model_dump_json(by_alias=True, exclude_unset=True)
