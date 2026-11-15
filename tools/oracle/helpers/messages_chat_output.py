import json
from collections.abc import Callable
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

from litellm.llms.anthropic.pass_through.adapters.transformation import (
    LiteLLMAnthropicMessagesAdapter as MessagesAdapter,
)
from litellm.llms.anthropic.pass_through.context_management.result import PolyfillResult
from litellm.types.llms.anthropic import (
    AppliedEdit,
    CompactionBlock,
    ContentJsonBlockDelta,
    ContentTextBlockDelta,
    ContentThinkingBlockDelta,
    StreamingContentBlockDeltaType,
    UsageIteration,
)
from litellm.types.llms.anthropic import ContentThinkingSignatureBlockDelta as SignatureDelta
from litellm.types.utils import (
    ChatCompletionDeltaToolCall,
    ChatCompletionMessageToolCall,
    Choices,
    Delta,
    Function,
    Message,
    ModelResponse,
    StreamingChoices,
    Usage,
)
from openai.types.chat.chat_completion_chunk import Choice as OpenAIStreamChoice
from pydantic import ConfigDict, JsonValue, TypeAdapter

ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)
ANSWER: Final = TypeAdapter(JsonValue)
NO_MESSAGES: Final[list[dict[str, JsonValue]]] = []
StreamChoices: TypeAlias = list[OpenAIStreamChoice | StreamingChoices]
ToolCalls: TypeAlias = list[ChatCompletionMessageToolCall] | list[ChatCompletionDeltaToolCall]
RecordValue: TypeAlias = str | ToolCalls | list[dict[str, JsonValue]] | dict[str, JsonValue] | None
# the pinned block union carries Any, so this alias names the JSON the block serializes as
BlockStart: TypeAlias = tuple[Literal["text", "tool_use", "thinking"], JsonValue]
ChunkDelta: TypeAlias = tuple[
    StreamingContentBlockDeltaType,
    ContentTextBlockDelta | ContentJsonBlockDelta | ContentThinkingBlockDelta | SignatureDelta,
]


@runtime_checkable
class Projection(Protocol):
    # ast-grep-ignore: no-long-names
    def _translate_streaming_openai_chunk_to_anthropic_content_block(
        self, choices: StreamChoices
    ) -> BlockStart: ...

    # ast-grep-ignore: no-long-names
    def translate_streaming_openai_response_to_anthropic(
        self,
        response: ModelResponse,
        current_content_block_index: int,
        applied_edits: list[AppliedEdit] | None = None,
    ) -> JsonValue: ...

    # ast-grep-ignore: no-long-names
    def translate_openai_response_to_anthropic(
        self,
        response: ModelResponse,
        tool_name_mapping: dict[str, str] | None = None,
        polyfill_result: PolyfillResult | None = None,
    ) -> JsonValue: ...

    def block_start(self, choices: StreamChoices) -> BlockStart:
        return self._translate_streaming_openai_chunk_to_anthropic_content_block(choices)


class Adapter(MessagesAdapter):
    # the base __init__ has no annotations, so a typed subclass avoids an untyped-call suppression
    def __init__(self) -> None:
        ...

    # the runtime protocol check wants the member; as a function it binds the adapter when called
    block_start: Callable[[Projection, StreamChoices], BlockStart] = Projection.block_start

    def chunk_delta(self, choices: StreamChoices) -> ChunkDelta:
        return self._translate_streaming_openai_chunk_to_anthropic(choices)


ADAPTER: Final = Adapter()
VIEW: Final[Projection] = TypeAdapter[Projection](
    Projection, config=ARBITRARY
).validate_python(ADAPTER)


class FunctionInput(TypedDict):
    name: str | None
    arguments: str | None


class MessageToolCallInput(TypedDict):
    id: str
    function: FunctionInput
    provider_specific_fields: dict[str, JsonValue] | None


class DeltaToolCallInput(TypedDict):
    id: str | None
    index: int
    function: FunctionInput


class RecordInput(TypedDict):
    content: str | None
    reasoning_content: str | None
    thinking_blocks: list[dict[str, JsonValue]] | None
    refusal: str | None
    provider_specific_fields: dict[str, JsonValue] | None


class MessageInput(RecordInput):
    tool_calls: list[MessageToolCallInput]


class DeltaInput(RecordInput):
    tool_calls: list[DeltaToolCallInput]


class ChoiceInput(TypedDict):
    finish_reason: str | None
    message: MessageInput


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


class ResponseInput(TypedDict):
    id: str
    model: str | None
    choices: list[ChoiceInput]
    usage: UsageInput | None


class PolyfillInput(TypedDict):
    applied_edits: list[AppliedEdit]
    compaction_block: CompactionBlock | None
    iterations_usage: list[UsageIteration] | None


class ResponseCall(TypedDict):
    operation: Literal["response"]
    response: ResponseInput
    tool_name_mapping: dict[str, str] | None
    polyfill_result: PolyfillInput | None


class StreamCall(TypedDict):
    operation: Literal["block", "delta"]
    choices: list[StreamChoiceInput]


class TerminalCall(TypedDict):
    operation: Literal["terminal"]
    choices: list[ChoiceInput]
    usage: UsageInput | None
    applied_edits: list[AppliedEdit]
    index: int


Input = ResponseCall | StreamCall | TerminalCall


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


def message_tool_call(tool: MessageToolCallInput) -> ChatCompletionMessageToolCall:
    function = Function(name=tool["function"]["name"], arguments=tool["function"]["arguments"])
    return ChatCompletionMessageToolCall(
        function=function, id=tool["id"], provider_specific_fields=tool["provider_specific_fields"]
    )


def delta_tool_call(tool: DeltaToolCallInput) -> ChatCompletionDeltaToolCall:
    function = Function(name=tool["function"]["name"], arguments=tool["function"]["arguments"])
    return ChatCompletionDeltaToolCall(index=tool["index"], id=tool["id"], function=function)


def record_values(value: RecordInput, tool_calls: ToolCalls) -> dict[str, RecordValue]:
    return {
        "content": value["content"], "reasoning_content": value["reasoning_content"],
        "thinking_blocks": value["thinking_blocks"], "refusal": value["refusal"],
        "tool_calls": tool_calls, "provider_specific_fields": value["provider_specific_fields"],
    }


def message_object(value: MessageInput) -> Message:
    tools = [message_tool_call(tool) for tool in value["tool_calls"]]
    return Message.model_validate(record_values(value, tools))


def delta_object(value: DeltaInput) -> Delta:
    tools = [delta_tool_call(tool) for tool in value["tool_calls"]]
    return Delta.model_validate(record_values(value, tools))


def choice_object(value: ChoiceInput) -> Choices:
    return Choices(
        finish_reason=value["finish_reason"], message=message_object(value["message"])
    )


def stream_choice(value: StreamChoiceInput) -> StreamingChoices:
    return StreamingChoices(
        finish_reason=value["finish_reason"], delta=delta_object(value["delta"])
    )


def response_object(value: ResponseInput) -> ModelResponse:
    return ModelResponse(
        id=value["id"],
        model=value["model"],
        choices=[choice_object(choice) for choice in value["choices"]],
        usage=usage_object(value["usage"]) if value["usage"] is not None else None,
    )


def terminal_response(call: TerminalCall) -> ModelResponse:
    return ModelResponse(
        choices=[choice_object(choice) for choice in call["choices"]],
        usage=usage_object(call["usage"]) if call["usage"] is not None else None,
    )


def answer_block(call: StreamCall) -> JsonValue:
    choices: StreamChoices = [stream_choice(choice) for choice in call["choices"]]
    block_type, block = VIEW.block_start(choices)
    return ANSWER.validate_json(json.dumps({"block_type": block_type, "block": block}))


def answer_delta(call: StreamCall) -> JsonValue:
    choices: StreamChoices = [stream_choice(choice) for choice in call["choices"]]
    return ANSWER.validate_json(json.dumps(ADAPTER.chunk_delta(choices)[1]))


def answer_terminal(call: TerminalCall) -> JsonValue:
    frame = VIEW.translate_streaming_openai_response_to_anthropic(
        terminal_response(call), call["index"], call["applied_edits"]
    )
    return ANSWER.validate_json(json.dumps(frame))


def polyfill_result(value: PolyfillInput | None) -> PolyfillResult | None:
    if value is None:
        return None
    return PolyfillResult(
        messages=NO_MESSAGES,
        system=None,
        applied_edits=value["applied_edits"],
        compaction_block=value["compaction_block"],
        iterations_usage=value["iterations_usage"],
    )


def answer_response(call: ResponseCall) -> JsonValue:
    translated = VIEW.translate_openai_response_to_anthropic(
        response_object(call["response"]), tool_name_mapping=call["tool_name_mapping"],
        polyfill_result=polyfill_result(call["polyfill_result"]),
    )
    return ANSWER.validate_json(json.dumps(translated))


def run(input_value: Input) -> JsonValue:
    match input_value["operation"]:
        case "response":
            return answer_response(input_value)
        case "block":
            return answer_block(input_value)
        case "delta":
            return answer_delta(input_value)
        case "terminal":
            return answer_terminal(input_value)
