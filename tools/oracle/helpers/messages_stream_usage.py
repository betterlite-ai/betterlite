import json
from typing import Final, Literal, TypeAlias, TypedDict, cast
from uuid import UUID

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.anthropic.pass_through.adapters.streaming_iterator import (
    AnthropicStreamWrapper,
)
from litellm.types.llms.anthropic import (
    AppliedEdit,
    CompactionBlock,
    ContentBlockDelta,
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
NORMALIZED_ID: Final = "msg_<uuid>"
ID_PREFIX: Final = "msg_"


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


class SnapshotStep(TypedDict):
    step: Literal["snapshot"]


class HoldStopStep(TypedDict):
    step: Literal["hold_stop"]
    event: MessageBlockDelta


class InitialUsageStep(TypedDict):
    step: Literal["initial_usage"]


class MergeUsageStep(TypedDict):
    step: Literal["merge_held_usage"]
    usage: UsageInput


class ChoicelessStep(TypedDict):
    step: Literal["choiceless"]
    usage: UsageInput | None


class AugmentStep(TypedDict):
    step: Literal["augment"]
    event: dict[str, JsonValue]


class RefusalStopStep(TypedDict):
    step: Literal["refusal_stop"]
    event: ContentBlockDelta | MessageBlockDelta


class CompactionStep(TypedDict):
    step: Literal["compaction"]
    calls: int


class DeltaContentStep(TypedDict):
    step: Literal["delta_has_content"]
    event: dict[str, JsonValue]


class IsBlankStep(TypedDict):
    step: Literal["is_blank"]
    choices: list[StreamChoiceInput]


class StartBlockStep(TypedDict):
    step: Literal["start_block"]
    choices: list[StreamChoiceInput]


StateStep: TypeAlias = SnapshotStep | HoldStopStep
UsageStep: TypeAlias = InitialUsageStep | MergeUsageStep | ChoicelessStep | AugmentStep
EventStep: TypeAlias = RefusalStopStep | CompactionStep
ChunkStep: TypeAlias = DeltaContentStep | IsBlankStep | StartBlockStep
Step: TypeAlias = StateStep | UsageStep | EventStep | ChunkStep


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
    if not value.startswith(ID_PREFIX):
        return value
    try:
        UUID(hex=value.removeprefix(ID_PREFIX))
    except ValueError:
        return value
    return NORMALIZED_ID


def usage_object(value: UsageInput) -> Usage:
    return Usage(
        prompt_tokens=value["prompt_tokens"],
        completion_tokens=value["completion_tokens"],
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
        content=value["content"], reasoning_content=value["reasoning_content"],
        tool_calls=tools, provider_specific_fields=value["provider_specific_fields"],
        refusal=value["refusal"],
    )
    if value["thinking_blocks"] is not None:
        delta["thinking_blocks"] = value["thinking_blocks"]
    return delta


def stream_choice(value: StreamChoiceInput) -> StreamingChoices:
    delta = delta_object(value["delta"])
    return StreamingChoices(finish_reason=value["finish_reason"], delta=delta)


def stream_chunk(choices: list[StreamChoiceInput]) -> ModelResponseStream:
    streamed: list[StreamingChoices] = [stream_choice(choice) for choice in choices]
    return ModelResponseStream(id=CHUNK_ID, created=0, choices=streamed)


class InertStream:
    def __init__(self, chunks: list[JsonValue] | None, messages: list[JsonValue] | None) -> None:
        self.chunks = chunks
        self.messages = messages

    def __iter__(self) -> "InertStream":
        return self

    def __aiter__(self) -> "InertStream":
        return self

    def __next__(self) -> ModelResponseStream:
        raise StopIteration

    async def __anext__(self) -> ModelResponseStream:
        raise StopAsyncIteration


class Wrapper(AnthropicStreamWrapper):
    def stream_state(self, logging_obj: Logging | None) -> JsonValue:
        messages = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L367-L378
            "list[JsonValue] | None", self.messages
        )
        chunks = self.chunks
        logged = None if logging_obj is None else logging_obj.streamed_anthropic_message_id
        return ANSWER.validate_json(
            json.dumps(
                {
                    "message_id": normalized_id(self._message_id),
                    "logged_message_id": None if logged is None else normalized_id(logged),
                    "logged_is_message_id": logged == self._message_id,
                    "model": self.model,
                    "index": self.current_content_block_index,
                    "type": self.current_content_block_type,
                    "start": self.current_content_block_start,
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
                    "held_content": self.holding_chunk,
                    "held_stop": self.holding_stop_reason_chunk,
                    "chunks_len": None if chunks is None else len(chunks),
                    "messages": messages,
                }
            )
        )

    def hold_stop(self, event: MessageBlockDelta) -> None:
        self.holding_stop_reason_chunk = event

    def initial_usage(self) -> JsonValue:
        return ANSWER.validate_json(json.dumps(self._create_initial_usage_delta()))

    def merge_held_usage(self, usage: Usage) -> JsonValue:
        chunk = ModelResponseStream(
            id=CHUNK_ID,
            created=0,
            choices=[],
            usage=usage,
        )
        merged = self._merge_usage_into_held_stop_reason_chunk(chunk)
        return ANSWER.validate_json(json.dumps(merged))

    def choiceless(self, usage: Usage | None) -> bool:
        chunk = ModelResponseStream(
            id=CHUNK_ID,
            created=0,
            choices=[],
            usage=usage,
        )
        return self._handle_choiceless_chunk(chunk)

    def augment(self, event: dict[str, JsonValue]) -> JsonValue:
        chunk = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L443-L458
            MessageBlockDelta, event
        )
        return ANSWER.validate_json(json.dumps(self._augment_message_delta_usage(chunk)))

    def refusal_stop(self, event: ContentBlockDelta | MessageBlockDelta) -> JsonValue:
        return ANSWER.validate_json(json.dumps(self._with_refusal_stop_details(event)))

    def compaction(self, calls: int) -> JsonValue:
        events = [self._next_compaction_event() for _ in range(calls)]
        return ANSWER.validate_json(json.dumps(events))

    def delta_has_content(self, event: dict[str, JsonValue]) -> bool:
        return self._delta_has_content(event)

    def is_blank(self, chunk: ModelResponseStream) -> bool:
        return self._is_blank_delta(chunk)

    def start_block(self, chunk: ModelResponseStream) -> bool:
        return self._should_start_new_content_block(chunk)


def state_answer(wrapper: Wrapper, logging_obj: Logging | None, step: Step) -> JsonValue:
    match step["step"]:
        case "snapshot":
            return wrapper.stream_state(logging_obj)
        case "hold_stop":
            wrapper.hold_stop(step["event"])
            return None
    raise ValueError("unknown state step")


def usage_answer(wrapper: Wrapper, step: Step) -> JsonValue:
    match step["step"]:
        case "initial_usage":
            return wrapper.initial_usage()
        case "merge_held_usage":
            return wrapper.merge_held_usage(usage_object(step["usage"]))
        case "choiceless":
            value = step["usage"]
            return wrapper.choiceless(None if value is None else usage_object(value))
        case "augment":
            return wrapper.augment(step["event"])
    raise ValueError("unknown usage step")


def event_answer(wrapper: Wrapper, step: Step) -> JsonValue:
    match step["step"]:
        case "refusal_stop":
            return wrapper.refusal_stop(step["event"])
        case "compaction":
            return wrapper.compaction(step["calls"])
    raise ValueError("unknown event step")


def chunk_answer(wrapper: Wrapper, step: Step) -> JsonValue:
    match step["step"]:
        case "delta_has_content":
            return wrapper.delta_has_content(step["event"])
        case "is_blank":
            return wrapper.is_blank(stream_chunk(step["choices"]))
        case "start_block":
            return wrapper.start_block(stream_chunk(step["choices"]))
    raise ValueError("unknown chunk step")


def answer_step(wrapper: Wrapper, logging_obj: Logging | None, step: Step) -> JsonValue:
    match step["step"]:
        case "snapshot" | "hold_stop":
            return state_answer(wrapper, logging_obj, step)
        case "initial_usage" | "merge_held_usage" | "choiceless" | "augment":
            return usage_answer(wrapper, step)
        case "refusal_stop" | "compaction":
            return event_answer(wrapper, step)
        case _:
            return chunk_answer(wrapper, step)


def build(input_value: Input) -> tuple[Wrapper, Logging | None]:
    logging_obj = object.__new__(Logging) if input_value["logging"] else None
    inert = InertStream(input_value["chunks_attr"], input_value["messages_attr"])
    wrapper = Wrapper(
        inert,
        model=input_value["model"],
        tool_name_mapping=input_value["tool_name_mapping"],
        applied_edits=input_value["applied_edits"],
        compaction_block=input_value["compaction_block"],
        iterations_usage=input_value["iterations_usage"],
        litellm_logging_obj=logging_obj,
    )
    return wrapper, logging_obj


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
