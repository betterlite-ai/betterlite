from collections.abc import AsyncIterator, Awaitable, Iterator
from typing import Protocol, TypeAlias

from betterlite._completion.wire import (
    CHUNK,
    RESPONSE,
    WireChoice,
    WireChunk,
    WireDelta,
    WireMessage,
    WireResponse,
    WireStreamChoice,
    WireUsage,
)
from betterlite.types.utils import (
    Choices,
    Delta,
    Message,
    ModelResponse,
    ModelResponseStream,
    StreamingChoices,
    Usage,
)
from betterlite.types.utils.base import InitValue


class ChunkSource(Protocol):
    def next_chunk(self) -> str | None: ...
    def anext_chunk(self) -> Awaitable[str | None]: ...
    def close(self) -> None: ...
    def aclose(self) -> Awaitable[None]: ...


def usage_from_wire(usage: WireUsage) -> Usage:
    return Usage(
        prompt_tokens=usage.get("prompt_tokens"),
        completion_tokens=usage.get("completion_tokens"),
        total_tokens=usage.get("total_tokens"),
        prompt_tokens_details=usage.get("prompt_tokens_details"),
        completion_tokens_details=usage.get("completion_tokens_details"),
    )


def message_from_wire(message: WireMessage) -> Message:
    # LiteLLM's Message carries refusal under provider_specific_fields even when it is absent
    fields = message.get("provider_specific_fields")
    if fields is None:
        fields = {"refusal": message.get("refusal")}
    return Message(
        content=message.get("content"), role=message["role"],
        function_call=message.get("function_call"), tool_calls=message.get("tool_calls"),
        audio=message.get("audio"), images=message.get("images"),
        annotations=message.get("annotations"),
        reasoning_content=message.get("reasoning_content"),
        thinking_blocks=message.get("thinking_blocks"),
        provider_specific_fields=fields,
    )


def choice_from_wire(choice: WireChoice) -> Choices:
    provider_fields = choice.get("provider_specific_fields")
    return Choices(
        finish_reason=choice.get("finish_reason"), index=choice["index"],
        message=message_from_wire(choice["message"]), logprobs=choice.get("logprobs"),
        provider_specific_fields={} if provider_fields is None else provider_fields,
    )


def response_from_wire(wire: WireResponse) -> ModelResponse:
    raw_usage = wire.get("usage")
    usage = None if raw_usage is None else usage_from_wire(raw_usage)
    return ModelResponse(
        id=wire["id"], created=wire["created"], model=wire["model"],
        system_fingerprint=wire.get("system_fingerprint"),
        usage=usage,
        choices=[choice_from_wire(choice) for choice in wire["choices"]],
        moderation=wire.get("moderation"), service_tier=wire.get("service_tier"),
    )


# ast-grep-ignore: no-one-line-functions
def response_from_text(text: str) -> ModelResponse:
    return response_from_wire(RESPONSE.validate_json(text))


def delta_from_wire(delta: WireDelta, first: bool) -> Delta:
    # LiteLLM writes the role on the first chunk and lets later chunks keep what the provider sent
    role: str | None = "assistant" if first else delta.get("role")
    return Delta(
        content=delta.get("content"), role=role,
        function_call=delta.get("function_call"), tool_calls=delta.get("tool_calls"),
        audio=delta.get("audio"), images=delta.get("images"),
        reasoning_content=delta.get("reasoning_content"),
        thinking_blocks=delta.get("thinking_blocks"),
        annotations=delta.get("annotations"),
        provider_specific_fields=delta.get("provider_specific_fields"),
    )


def streaming_choice(choice: WireStreamChoice, first: bool) -> StreamingChoices:
    return StreamingChoices(
        finish_reason=choice.get("finish_reason"), index=choice["index"],
        delta=delta_from_wire(choice["delta"], first), logprobs=choice.get("logprobs"),
    )


def chunk_from_wire(wire: WireChunk, first: bool) -> ModelResponseStream:
    extras: dict[str, InitValue] = {
        "model": wire["model"], "system_fingerprint": wire.get("system_fingerprint"),
        "moderation": None, "service_tier": None,
    }
    usage = wire.get("usage")
    if usage is not None:
        extras["usage"] = usage_from_wire(usage)
    raw_choices = wire.get("choices")
    choices: list[StreamingChoices] = (
        [] if raw_choices is None else [streaming_choice(choice, first) for choice in raw_choices]
    )
    return ModelResponseStream(
        choices=choices, id=wire["id"], created=wire["created"],
        provider_specific_fields=wire.get("provider_specific_fields"), **extras,
    )


class CustomStreamWrapper:
    source: ChunkSource
    closed: bool
    sent_first: bool

    def __init__(self, source: ChunkSource) -> None:
        self.source = source
        self.closed = False
        self.sent_first = False

    def __iter__(self) -> Iterator[ModelResponseStream]:
        return self

    def _chunk(self, text: str) -> ModelResponseStream:
        # only the first chunk carries the assistant role LiteLLM writes
        first = not self.sent_first
        self.sent_first = True
        return chunk_from_wire(CHUNK.validate_json(text), first)

    def __next__(self) -> ModelResponseStream:
        if self.closed:
            raise StopIteration
        text = self.source.next_chunk()
        if text is None:
            raise StopIteration
        return self._chunk(text)

    def __aiter__(self) -> AsyncIterator[ModelResponseStream]:
        return self

    async def __anext__(self) -> ModelResponseStream:
        if self.closed:
            raise StopAsyncIteration
        text = await self.source.anext_chunk()
        if text is None:
            raise StopAsyncIteration
        return self._chunk(text)

    def close(self) -> None:
        self.closed = True
        self.source.close()

    async def aclose(self) -> None:
        self.closed = True
        await self.source.aclose()


CompletionOutcome: TypeAlias = ModelResponse | CustomStreamWrapper
