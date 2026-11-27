from typing import Literal, TypeAlias

from pydantic import JsonValue
from typing_extensions import Required, TypedDict, override

from betterlite.types.llms.base import BaseLiteLLMOpenAIResponseObject
from betterlite.types.llms.openai.core.chat_blocks import (
    ChatCompletionAnnotation,
    ChatCompletionDeltaToolCallChunk,
    ChatCompletionRedactedThinkingBlock,
    ChatCompletionThinkingBlock,
    ChatCompletionToolCallChunk,
    ChatCompletionToolCallFunctionChunk,
)
from betterlite.types.utils.base import InitValue

ChatCompletionAssistantContentValue: TypeAlias = str  # ast-grep-ignore: no-long-names


class ChatCompletionDeltaChunk(TypedDict, total=False):
    content: str | None
    tool_calls: list[ChatCompletionDeltaToolCallChunk]  # ast-grep-ignore: no-long-names
    role: str


class ChatCompletionResponseMessage(TypedDict, total=False):
    content: ChatCompletionAssistantContentValue | None  # ast-grep-ignore: no-long-names
    annotations: list[ChatCompletionAnnotation] | None
    tool_calls: list[ChatCompletionToolCallChunk] | None
    role: Literal["assistant"]
    function_call: ChatCompletionToolCallFunctionChunk | None  # ast-grep-ignore: no-long-names
    provider_specific_fields: dict[str, JsonValue] | None
    reasoning_content: str | None
    thinking_blocks: (
        list[
            ChatCompletionThinkingBlock
            # ast-grep-ignore: no-long-names
            | ChatCompletionRedactedThinkingBlock
        ]
        | None
    )


class ChatCompletionUsageBlock(TypedDict, total=False):
    prompt_tokens: Required[int]
    completion_tokens: Required[int]
    total_tokens: Required[int]
    prompt_tokens_details: dict[str, JsonValue] | None
    completion_tokens_details: dict[str, JsonValue] | None


# ast-grep-ignore: no-long-names
class OpenAIChatCompletionChunk(BaseLiteLLMOpenAIResponseObject):
    id: str
    choices: list[JsonValue]
    created: int
    model: str
    object: Literal["chat.completion.chunk"]
    moderation: JsonValue = None
    service_tier: str | None = None
    system_fingerprint: str | None = None
    usage: JsonValue = None

    @override
    def __init__(self, **data: InitValue) -> None:
        data["object"] = "chat.completion.chunk"
        super().__init__(**data)
