import asyncio
import logging
from collections.abc import AsyncIterator
from datetime import UTC, datetime
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.anthropic.pass_through.messages.agentic_streaming_iterator import (
    AgenticAnthropicStreamingIterator as AgenticIterator,
)
from litellm.llms.anthropic.pass_through.messages.transformation import AnthropicMessagesConfig
from pydantic import JsonValue
from typing_extensions import override

PREFIX: Final = "AgenticStreamingIterator:"
CALL_ID: Final = "call-1"


class Reply(TypedDict):
    kind: Literal["absent", "stream"]
    chunks: list[str]


Step: TypeAlias = Literal["read", "read_all", "release_drain", "release_hook", "close"]


class Input(TypedDict):
    model: str
    chunks: list[str]
    hold_back: bool
    server_fulfilled_tool_names: list[str]
    ping_interval_seconds: float
    reply: Reply
    steps: list[Step]


class HookCall(TypedDict):
    model: str
    response: JsonValue


class Answer(TypedDict):
    results: list[JsonValue]
    buffered: list[JsonValue]
    hook_calls: list[HookCall]
    effects: list[JsonValue]
    logs: list[JsonValue]


class Lines(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.lines: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if message.startswith(PREFIX):
            self.lines.append({"level": record.levelname, "message": message})


class ScriptedStream(AsyncIterator[bytes]):
    def __init__(
        self, chunks: list[bytes], gate: asyncio.Event, effects: list[JsonValue], close_effect: str
    ) -> None:
        self._chunks = chunks
        self._gate = gate
        self._effects = effects
        self._close_effect = close_effect
        self._index = 0

    @override
    async def __anext__(self) -> bytes:
        await self._gate.wait()
        if self._index >= len(self._chunks):
            raise StopAsyncIteration
        chunk = self._chunks[self._index]
        self._index += 1
        return chunk

    async def aclose(self) -> None:
        self._effects.append(self._close_effect)


class ScriptedHandler:
    def __init__(self, reply: Reply, gate: asyncio.Event, effects: list[JsonValue]) -> None:
        self._reply = reply
        self._gate = gate
        self._effects = effects
        self.calls: list[HookCall] = []

    async def _call_agentic_completion_hooks(
        self,
        *,
        response: JsonValue,
        model: str,
        **_: JsonValue,
    ) -> JsonValue | ScriptedStream:
        self.calls.append({"model": model, "response": response})
        try:
            await self._gate.wait()
        except asyncio.CancelledError:
            self._effects.append("hook-cancelled")
            raise
        if self._reply["kind"] == "absent":
            return None
        ready = asyncio.Event()
        ready.set()
        chunks = [chunk.encode() for chunk in self._reply["chunks"]]
        return ScriptedStream(chunks, ready, self._effects, "followup-close")


def logging_object(model: str) -> Logging:
    return Logging(
        model=model,
        messages=None,
        stream=True,
        call_type="acompletion",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id=CALL_ID,
        function_id="fn-1",
    )


async def _read_one(iterator: AgenticIterator, answer: Answer) -> bool:
    try:
        chunk = await iterator.__anext__()
    except StopAsyncIteration:
        answer["results"].append(None)
        answer["buffered"].append(iterator.has_buffered_provider_output)
        return False
    answer["results"].append(chunk.decode())
    answer["buffered"].append(iterator.has_buffered_provider_output)
    return True


async def _drive(input_value: Input) -> Answer:
    effects: list[JsonValue] = []
    drain_gate = asyncio.Event()
    hook_gate = asyncio.Event()
    handler = ScriptedHandler(input_value["reply"], hook_gate, effects)
    chunks = [chunk.encode() for chunk in input_value["chunks"]]
    no_messages: list[dict[str, str]] = []
    no_params: dict[str, str] = {}
    no_kwargs: dict[str, str] = {}
    iterator = AgenticIterator(
        completion_stream=ScriptedStream(chunks, drain_gate, effects, "inner-close"),
        http_handler=handler,
        model=input_value["model"],
        messages=no_messages,
        anthropic_messages_provider_config=AnthropicMessagesConfig(),
        anthropic_messages_optional_request_params=no_params,
        logging_obj=logging_object(input_value["model"]),
        custom_llm_provider="anthropic",
        kwargs=no_kwargs,
        hold_back=input_value["hold_back"],
        server_fulfilled_tool_names=frozenset(input_value["server_fulfilled_tool_names"]),
        ping_interval_seconds=input_value["ping_interval_seconds"],
    )
    answer: Answer = {
        "results": [],
        "buffered": [],
        "hook_calls": handler.calls,
        "effects": effects,
        "logs": [],
    }
    for step in input_value["steps"]:
        if step == "read":
            await _read_one(iterator, answer)
        elif step == "read_all":
            more = True
            while more:
                more = await _read_one(iterator, answer)
        elif step == "release_drain":
            drain_gate.set()
        elif step == "release_hook":
            hook_gate.set()
        elif step == "close":
            await iterator.aclose()
    return answer


def run(input_value: Input) -> Answer:
    handler = Lines()
    logger = logging.getLogger("LiteLLM")
    previous_level = logger.level
    previous_handlers = list(logger.handlers)
    for existing in previous_handlers:
        logger.removeHandler(existing)
    logger.addHandler(handler)
    logger.setLevel(logging.DEBUG)
    try:
        answer = asyncio.run(_drive(input_value))
    finally:
        logger.removeHandler(handler)
        for existing in previous_handlers:
            logger.addHandler(existing)
        logger.setLevel(previous_level)
    answer["logs"] = handler.lines
    return answer
