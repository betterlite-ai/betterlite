import json
from collections.abc import AsyncIterator, Iterator
from datetime import UTC, datetime
from typing import Final, Literal, TypedDict, cast
from uuid import UUID

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.anthropic.pass_through.adapters.streaming_iterator import (
    AnthropicStreamWrapper,
)
from litellm.types.llms.anthropic import (
    AppliedEdit,
    CompactionBlock,
    MessageBlockDelta,
    UsageIteration,
)
from litellm.types.utils import (
    ChatCompletionDeltaToolCall,
    Delta,
    Function,
    ModelResponseStream,
    StreamingChoices,
    Usage,
)
from pydantic import JsonValue, TypeAdapter

ANSWER: Final = TypeAdapter(JsonValue)
CHUNK_ID: Final = "usage-chunk"


class FunctionInput(TypedDict):
    name: str | None
    arguments: str | None


class DeltaToolCallInput(TypedDict):
    id: str | None
    index: int
    function: FunctionInput


class DeltaInput(TypedDict):
    content: str | None
    tool_calls: list[DeltaToolCallInput]
    reasoning_content: str | None
    thinking_blocks: list[dict[str, JsonValue]] | None
    refusal: str | None
    provider_specific_fields: dict[str, JsonValue] | None


class StreamChoiceInput(TypedDict):
    finish_reason: str | None
    delta: DeltaInput


class UsageInput(TypedDict):
    prompt_tokens: int | None
    completion_tokens: int | None
    prompt_tokens_details: dict[str, JsonValue] | None
    server_tool_use: dict[str, JsonValue] | None
    cache_read_input_tokens: JsonValue
    cache_creation_input_tokens: JsonValue
    _cache_read_input_tokens: JsonValue
    _cache_creation_input_tokens: JsonValue


class Step(TypedDict, total=False):
    step: Literal[
        "snapshot", "hold_stop", "initial_usage", "merge_held_usage", "choiceless",
        "augment", "refusal_stop", "compaction", "delta_has_content", "is_blank", "start_block",
    ]
    event: MessageBlockDelta
    usage: UsageInput | None
    calls: int
    choices: list[StreamChoiceInput]


class Input(TypedDict):
    logging: bool
    model: str
    tool_name_mapping: dict[str, str]
    applied_edits: list[AppliedEdit]
    compaction_block: CompactionBlock | None
    iterations_usage: list[UsageIteration] | None
    chunks_attr: list[JsonValue] | None
    messages_attr: list[JsonValue] | None
    steps: list[Step]


def normalized_id(value: str) -> str:
    if not value.startswith("msg_"):
        return value
    try:
        UUID(hex=value.removeprefix("msg_"))
    except ValueError:
        return value
    return "msg_<uuid>"


def usage_object(value: UsageInput) -> Usage:
    return Usage(
        prompt_tokens=value["prompt_tokens"], completion_tokens=value["completion_tokens"],
        prompt_tokens_details=value["prompt_tokens_details"],
        server_tool_use=value["server_tool_use"],
        cache_read_input_tokens=value["cache_read_input_tokens"],
        cache_creation_input_tokens=value["cache_creation_input_tokens"],
        _cache_read_input_tokens=value["_cache_read_input_tokens"],
        _cache_creation_input_tokens=value["_cache_creation_input_tokens"],
    )


def delta_tool_call(tool: DeltaToolCallInput) -> ChatCompletionDeltaToolCall:
    function = Function(name=tool["function"]["name"], arguments=tool["function"]["arguments"])
    return ChatCompletionDeltaToolCall(index=tool["index"], id=tool["id"], function=function)


def delta_object(value: DeltaInput) -> Delta:
    tools = [delta_tool_call(tool) for tool in value["tool_calls"]]
    delta = Delta(
        content=value["content"],
        reasoning_content=value["reasoning_content"],
        tool_calls=tools,
        provider_specific_fields=value["provider_specific_fields"],
        refusal=value["refusal"],
    )
    if value["thinking_blocks"] is not None:
        delta["thinking_blocks"] = value["thinking_blocks"]
    return delta


def stream_chunk(choices: list[StreamChoiceInput]) -> ModelResponseStream:
    streamed: list[StreamingChoices] = [
        StreamingChoices(finish_reason=choice["finish_reason"], delta=delta_object(choice["delta"]))
        for choice in choices
    ]
    return ModelResponseStream(id=CHUNK_ID, created=0, choices=streamed)


class InertStream:
    def __init__(self, chunks: list[JsonValue] | None, messages: list[JsonValue] | None) -> None:
        self.chunks = chunks
        self.messages = messages

    def __iter__(self) -> Iterator[ModelResponseStream]:
        return iter(())

    def __aiter__(self) -> AsyncIterator[ModelResponseStream]:
        return self

    async def __anext__(self) -> ModelResponseStream:
        raise StopAsyncIteration


class Wrapper(AnthropicStreamWrapper):
    def stream_state(self, logging_obj: Logging | None) -> JsonValue:
        messages = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L375-L376
            "list[JsonValue] | None", self.messages
        )
        start = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L363-L366
            "JsonValue", self.current_content_block_start
        )
        held_content = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L315
            "JsonValue", self.holding_chunk
        )
        chunks = self.chunks
        logged = None if logging_obj is None else logging_obj.streamed_anthropic_message_id
        snapshot: dict[str, object] = {
            "message_id": normalized_id(self._message_id),
            "logged_message_id": None if logged is None else normalized_id(logged),
            "logged_is_message_id": logged == self._message_id,
            "model": self.model, "index": self.current_content_block_index,
            "type": self.current_content_block_type,
            "start": start,
            "sent_first_chunk": self.sent_first_chunk,
            "sent_last_message": self.sent_last_message,
            "sent_content_block_start": self.sent_content_block_start,
            "sent_content_block_finish": self.sent_content_block_finish,
            "queued_usage_chunk": self.queued_usage_chunk,
            "sent_compaction_block": self.sent_compaction_block,
            "sent_compaction_block_start": self.sent_compaction_block_start,
            "sent_compaction_block_delta": self.sent_compaction_block_delta,
            "refusal_text": self._refusal_text,
            "tool_name_mapping": self.tool_name_mapping,
            "applied_edits": self.applied_edits,
            "compaction_block": self.compaction_block,
            "iterations_usage": self.iterations_usage,
            "held_content": held_content,
            "held_stop": self.holding_stop_reason_chunk,
            "chunks_len": None if chunks is None else len(chunks),
            "messages": messages,
        }
        return ANSWER.validate_json(json.dumps(snapshot))

    def hold_stop(self, event: MessageBlockDelta) -> None:
        self.holding_stop_reason_chunk = event

    def initial_usage(self) -> JsonValue:
        return ANSWER.validate_json(json.dumps(self._create_initial_usage_delta()))

    def merge_held_usage(self, usage: Usage) -> JsonValue:
        chunk = ModelResponseStream(id=CHUNK_ID, created=0, choices=[], usage=usage)
        merged = self._merge_usage_into_held_stop_reason_chunk(chunk)
        return ANSWER.validate_json(json.dumps(merged))

    def choiceless(self, usage: Usage | None) -> bool:
        chunk = ModelResponseStream(id=CHUNK_ID, created=0, choices=[], usage=usage)
        return self._handle_choiceless_chunk(chunk)

    def event_step(self, name: str, event: MessageBlockDelta) -> JsonValue:
        if name == "augment":
            return ANSWER.validate_json(json.dumps(self._augment_message_delta_usage(event)))
        chunk = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L1049-L1067
            "MessageBlockDelta", self._with_refusal_stop_details(event)
        )
        return ANSWER.validate_json(json.dumps(chunk))

    def compaction(self, calls: int) -> JsonValue:
        events = [self._next_compaction_event() for _ in range(calls)]
        return ANSWER.validate_json(json.dumps(events))

    def delta_has_content(self, event: MessageBlockDelta) -> bool:
        return self._delta_has_content(event)

    def choice_step(self, name: str, choices: list[StreamChoiceInput]) -> bool:
        chunk = stream_chunk(choices)
        if name == "is_blank":
            return self._is_blank_delta(chunk)
        return self._should_start_new_content_block(chunk)

    def usage_step(self, name: str, usage: UsageInput | None) -> JsonValue:
        if usage is None:
            return self.choiceless(None)
        if name == "choiceless":
            return self.choiceless(usage_object(usage))
        return self.merge_held_usage(usage_object(usage))


def logging_object() -> Logging:
    return Logging(
        model="claude", messages=None, stream=False, call_type="anthropic_messages",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1", function_id="fn-1",
    )


def build(input_value: Input) -> tuple[Wrapper, Logging | None]:
    logging_obj = logging_object() if input_value["logging"] else None
    inert = InertStream(input_value["chunks_attr"], input_value["messages_attr"])
    wrapper = Wrapper(
        inert,
        model=input_value["model"], tool_name_mapping=input_value["tool_name_mapping"],
        applied_edits=input_value["applied_edits"],
        compaction_block=input_value["compaction_block"],
        iterations_usage=input_value["iterations_usage"], litellm_logging_obj=logging_obj,
    )
    return wrapper, logging_obj


def answer_step(wrapper: Wrapper, logging_obj: Logging | None, step: Step) -> JsonValue:
    answer: JsonValue = None
    name = step.get("step")
    match name:
        case "snapshot":
            answer = wrapper.stream_state(logging_obj)
        case "hold_stop":
            if (event := step.get("event")) is not None:
                wrapper.hold_stop(event)
        case "initial_usage":
            answer = wrapper.initial_usage()
        case "merge_held_usage" | "choiceless":
            answer = wrapper.usage_step(name, step.get("usage"))
        case "augment" | "refusal_stop":
            if (event := step.get("event")) is not None:
                answer = wrapper.event_step(name, event)
        case "compaction":
            if (calls := step.get("calls")) is not None:
                answer = wrapper.compaction(calls)
        case "delta_has_content":
            if (event := step.get("event")) is not None:
                answer = wrapper.delta_has_content(event)
        case "is_blank" | "start_block":
            if (choices := step.get("choices")) is not None:
                answer = wrapper.choice_step(name, choices)
        case _:
            pass
    return answer


def run(input_value: Input) -> JsonValue:
    wrapper, logging_obj = build(input_value)
    answers = [
        {
            "answer": answer_step(wrapper, logging_obj, step),
            "state": wrapper.stream_state(logging_obj),
        }
        for step in input_value["steps"]
    ]
    return ANSWER.validate_json(json.dumps(answers))
