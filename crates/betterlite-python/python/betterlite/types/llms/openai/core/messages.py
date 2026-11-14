from collections.abc import Iterable
from typing import Final, Literal, Required, TypeAlias, TypedDict

from pydantic import JsonValue

from betterlite.types.llms.openai.core.aliases import AllPromptValues
from betterlite.types.llms.openai.core.chat_blocks import (
    ChatCompletionAssistantToolCall,
    ChatCompletionCachedContent,
    ChatCompletionImageObject,
    ChatCompletionReasoningItem,
    ChatCompletionRedactedThinkingBlock,
    ChatCompletionTextObject,
    ChatCompletionThinkingBlock,
    ChatCompletionToolCallFunctionChunk,
    OpenAIMessageContent,
)


# ast-grep-ignore: no-long-names
class OpenAIChatCompletionUserMessage(TypedDict):
    role: Literal["user"]
    content: OpenAIMessageContent


# ast-grep-ignore: no-long-names
class OpenAITextCompletionUserMessage(TypedDict):
    role: Literal["user"]
    content: AllPromptValues


# ast-grep-ignore: no-long-names
class ChatCompletionUserMessage(OpenAIChatCompletionUserMessage, total=False):
    cache_control: ChatCompletionCachedContent


# ast-grep-ignore: no-long-names
class OpenAIChatCompletionAssistantMessage(TypedDict, total=False):
    role: Required[Literal["assistant"]]
    content: (
        str
        | Iterable[
            ChatCompletionTextObject
            | ChatCompletionThinkingBlock
            # ast-grep-ignore: no-long-names
            | ChatCompletionRedactedThinkingBlock
            | ChatCompletionImageObject
        ]
        | None
    )
    name: str | None
    tool_calls: list[ChatCompletionAssistantToolCall] | None  # ast-grep-ignore: no-long-names
    function_call: ChatCompletionToolCallFunctionChunk | None  # ast-grep-ignore: no-long-names
    reasoning_content: str | None


# ast-grep-ignore: no-long-names
class ChatCompletionAssistantMessage(OpenAIChatCompletionAssistantMessage, total=False):
    cache_control: ChatCompletionCachedContent
    thinking_blocks: (
        # ast-grep-ignore: no-long-names
        list[ChatCompletionThinkingBlock | ChatCompletionRedactedThinkingBlock] | None
    )
    reasoning_items: list[ChatCompletionReasoningItem] | None


# ast-grep-ignore: no-long-names
class ChatCompletionToolReferenceObject(TypedDict):
    type: Literal["tool_reference"]
    tool_name: str


ToolMessageContentPart: TypeAlias = (
    # ast-grep-ignore: no-long-names
    ChatCompletionTextObject | ChatCompletionImageObject | ChatCompletionToolReferenceObject
)


class ChatCompletionToolMessage(TypedDict):
    role: Literal["tool"]
    content: str | Iterable[ToolMessageContentPart]
    tool_call_id: str


class ChatCompletionFunctionMessage(TypedDict):
    role: Literal["function"]
    content: str | Iterable[ChatCompletionTextObject] | None
    name: str
    tool_call_id: str | None


# ast-grep-ignore: no-long-names
class OpenAIChatCompletionSystemMessage(TypedDict, total=False):
    role: Required[Literal["system"]]
    content: Required[str | list[JsonValue]]
    name: str


# ast-grep-ignore: no-long-names
class OpenAIChatCompletionDeveloperMessage(TypedDict, total=False):
    role: Required[Literal["developer"]]
    content: Required[str | list[JsonValue]]
    name: str


# ast-grep-ignore: no-long-names
class ChatCompletionSystemMessage(OpenAIChatCompletionSystemMessage, total=False):
    cache_control: ChatCompletionCachedContent


# ast-grep-ignore: no-long-names
class ChatCompletionDeveloperMessage(OpenAIChatCompletionDeveloperMessage, total=False):
    cache_control: ChatCompletionCachedContent


class GenericChatCompletionMessage(TypedDict, total=False):
    role: Required[str]
    content: Required[str | list[JsonValue]]


ValidUserMessageContentTypes: Final = [
    "text",
    "image_url",
    "input_audio",
    "audio_url",
    "document",
    "guarded_text",
    "grounding_source",
    "query",
    "video_url",
    "file",
]

# ast-grep-ignore: no-long-names
ValidUserMessageContentTypesLiteral: TypeAlias = Literal[
    "text",
    "image_url",
    "input_audio",
    "audio_url",
    "document",
    "guarded_text",
    "grounding_source",
    "query",
    "video_url",
    "file",
]

# ast-grep-ignore: no-long-names
ValidAssistantMessageContentTypesLiteral: TypeAlias = Literal[
    "text",
    "thinking",
    "redacted_thinking",
    "image_url",
]

# ast-grep-ignore: no-long-names
ValidAssistantMessageContentTypes: Final = [
    "text",
    "thinking",
    "redacted_thinking",
    "image_url",
]

# ast-grep-ignore: no-long-names
ValidChatCompletionMessageContentTypesLiteral: TypeAlias = Literal[
    "text",
    "image_url",
    "input_audio",
    "audio_url",
    "document",
    "guarded_text",
    "grounding_source",
    "query",
    "video_url",
    "file",
    "thinking",
    "redacted_thinking",
]

# ast-grep-ignore: no-long-names
ValidChatCompletionMessageContentTypes: Final = [
    "text",
    "image_url",
    "input_audio",
    "audio_url",
    "document",
    "guarded_text",
    "grounding_source",
    "query",
    "video_url",
    "file",
    "thinking",
    "redacted_thinking",
]

AllMessageValues: TypeAlias = (
    ChatCompletionUserMessage
    | ChatCompletionAssistantMessage
    | ChatCompletionToolMessage
    | ChatCompletionSystemMessage
    | ChatCompletionFunctionMessage
    | ChatCompletionDeveloperMessage
)
