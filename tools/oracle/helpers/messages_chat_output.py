import json
from typing import Final, Literal, TypedDict

from litellm.llms.anthropic.pass_through.adapters.transformation import (
    LiteLLMAnthropicMessagesAdapter as MessagesAdapter,
)
from litellm.llms.anthropic.pass_through.context_management.result import PolyfillResult
from litellm.types.llms.anthropic import (
    AppliedEdit,
    CompactionBlock,
    ContentBlockContentBlockDict,
    ContentBlockDelta,
    ContentJsonBlockDelta,
    ContentTextBlockDelta,
    ContentThinkingBlockDelta,
    ContentThinkingSignatureBlockDelta as SignatureDelta,
    MessageBlockDelta,
    StreamingContentBlockDeltaType,
    UsageIteration,
)
from litellm.types.llms.openai import (
    ChatCompletionRedactedThinkingBlock as RedactedBlock,
    ChatCompletionThinkingBlock,
)
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
from pydantic import JsonValue as Json
from pydantic import TypeAdapter


class Adapter(MessagesAdapter):
    # the base __init__ has no annotations, so a typed subclass avoids an untyped-call suppression
    def __init__(self) -> None:
        ...

    def block_start(
        self,
        choices: list[OpenAIStreamChoice | StreamingChoices],
    ) -> tuple[Literal["text", "tool_use", "thinking"], ContentBlockContentBlockDict]:
        return self._translate_streaming_openai_chunk_to_anthropic_content_block(choices)

    def chunk_delta(
        self,
        choices: list[OpenAIStreamChoice | StreamingChoices],
    ) -> tuple[
        StreamingContentBlockDeltaType,
        ContentTextBlockDelta
        | ContentJsonBlockDelta
        | ContentThinkingBlockDelta
        | SignatureDelta,
    ]:
        return self._translate_streaming_openai_chunk_to_anthropic(choices)

    def stream_frame(
        self,
        response: ModelResponse,
        index: int,
        applied_edits: list[AppliedEdit] | None,
    ) -> ContentBlockDelta | MessageBlockDelta:
        return self.translate_streaming_openai_response_to_anthropic(response, index, applied_edits)


ADAPTER: Final = Adapter()
ANSWER: Final = TypeAdapter(Json)


class FunctionInput(TypedDict):
    name: str | None
    arguments: str | None


class MessageToolCallInput(TypedDict):
    id: str
    function: FunctionInput
    provider_specific_fields: dict[str, Json] | None


class DeltaToolCallInput(TypedDict):
    id: str | None
    index: int
    function: FunctionInput


class MessageInput(TypedDict):
    content: str | None
    tool_calls: list[MessageToolCallInput]
    reasoning_content: str | None
    thinking_blocks: list[ChatCompletionThinkingBlock | RedactedBlock] | None
    refusal: str | None
    provider_specific_fields: dict[str, Json] | None


class DeltaInput(TypedDict):
    content: str | None
    tool_calls: list[DeltaToolCallInput]
    reasoning_content: str | None
    thinking_blocks: list[ChatCompletionThinkingBlock | RedactedBlock] | None
    refusal: str | None
    provider_specific_fields: dict[str, Json] | None


class ChoiceInput(TypedDict):
    finish_reason: str | None
    message: MessageInput


class StreamChoiceInput(TypedDict):
    finish_reason: str | None
    delta: DeltaInput


class UsageInput(TypedDict):
    prompt_tokens: int | None
    completion_tokens: int | None
    prompt_tokens_details: dict[str, Json] | None
    server_tool_use: dict[str, Json] | None
    cache_read_input_tokens: Json
    cache_creation_input_tokens: Json
    _cache_read_input_tokens: Json
    _cache_creation_input_tokens: Json


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


class BlockCall(TypedDict):
    operation: Literal["block"]
    choices: list[StreamChoiceInput]


class DeltaCall(TypedDict):
    operation: Literal["delta"]
    choices: list[StreamChoiceInput]


class TerminalCall(TypedDict):
    operation: Literal["terminal"]
    choices: list[ChoiceInput]
    usage: UsageInput | None
    applied_edits: list[AppliedEdit]
    index: int


Input = ResponseCall | BlockCall | DeltaCall | TerminalCall


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


def message_object(value: MessageInput) -> Message:
    tool_calls: list[ChatCompletionMessageToolCall] = []
    for tool in value["tool_calls"]:
        tool_calls.append(
            ChatCompletionMessageToolCall(
                function=Function(name=tool["function"]["name"], arguments=tool["function"]["arguments"]),
                id=tool["id"],
                provider_specific_fields=tool["provider_specific_fields"],
            )
        )
    return Message(
        content=value["content"],
        tool_calls=tool_calls,
        reasoning_content=value["reasoning_content"],
        thinking_blocks=value["thinking_blocks"],
        refusal=value["refusal"],
        provider_specific_fields=value["provider_specific_fields"],
    )


def delta_object(value: DeltaInput) -> Delta:
    tool_calls: list[ChatCompletionDeltaToolCall] = []
    for tool in value["tool_calls"]:
        tool_calls.append(
            ChatCompletionDeltaToolCall(
                index=tool["index"],
                id=tool["id"],
                function=Function(name=tool["function"]["name"], arguments=tool["function"]["arguments"]),
            )
        )
    return Delta(
        content=value["content"],
        tool_calls=tool_calls,
        reasoning_content=value["reasoning_content"],
        thinking_blocks=value["thinking_blocks"],
        refusal=value["refusal"],
        provider_specific_fields=value["provider_specific_fields"],
    )


def choice_object(value: ChoiceInput) -> Choices:
    return Choices(
        finish_reason=value["finish_reason"],
        message=message_object(value["message"]),
    )


def stream_choice(value: StreamChoiceInput) -> StreamingChoices:
    return StreamingChoices(
        finish_reason=value["finish_reason"],
        delta=delta_object(value["delta"]),
    )


def response_object(value: ResponseInput) -> ModelResponse:
    usage_value = value["usage"]
    choices = [choice_object(choice) for choice in value["choices"]]
    return ModelResponse(
        id=value["id"],
        model=value["model"],
        choices=choices,
        usage=usage_object(usage_value) if usage_value is not None else None,
    )


def answer_block(call: BlockCall) -> Json:
    choices: list[OpenAIStreamChoice | StreamingChoices] = []
    for choice in call["choices"]:
        choices.append(stream_choice(choice))
    block_type, block = ADAPTER.block_start(choices)
    return ANSWER.validate_json(json.dumps({"block_type": block_type, "block": block}))


def answer_delta(call: DeltaCall) -> Json:
    choices: list[OpenAIStreamChoice | StreamingChoices] = []
    for choice in call["choices"]:
        choices.append(stream_choice(choice))
    projected = ADAPTER.chunk_delta(choices)
    return ANSWER.validate_json(json.dumps(projected[1]))


def answer_terminal(call: TerminalCall) -> Json:
    usage_value = call["usage"]
    choices = [choice_object(choice) for choice in call["choices"]]
    response = ModelResponse(
        choices=choices,
        usage=usage_object(usage_value) if usage_value is not None else None,
    )
    frame = ADAPTER.stream_frame(response, call["index"], call["applied_edits"])
    return ANSWER.validate_json(json.dumps(frame))


def answer_response(call: ResponseCall) -> Json:
    polyfill_value = call["polyfill_result"]
    polyfill = (
        PolyfillResult(
            messages=[],
            system=None,
            applied_edits=polyfill_value["applied_edits"],
            compaction_block=polyfill_value["compaction_block"],
            iterations_usage=polyfill_value["iterations_usage"],
        )
        if polyfill_value is not None
        else None
    )
    translated = ADAPTER.translate_openai_response_to_anthropic(
        response_object(call["response"]),
        tool_name_mapping=call["tool_name_mapping"],
        polyfill_result=polyfill,
    )
    return ANSWER.validate_json(json.dumps(translated))


def run(input_value: Input) -> Json:
    match input_value["operation"]:
        case "response":
            return answer_response(input_value)
        case "block":
            return answer_block(input_value)
        case "delta":
            return answer_delta(input_value)
        case "terminal":
            return answer_terminal(input_value)
