from collections.abc import Iterable, Sequence
from typing import Literal, TypeAlias

from pydantic import JsonValue
from typing_extensions import ReadOnly, Required, TypedDict

from betterlite.types.llms.anthropic.tools import ToolCaller, ToolReference
from betterlite.types.llms.openai.core.chat_blocks import (
    ChatCompletionCachedContent,
    ChatCompletionRedactedThinkingBlock,
    ChatCompletionThinkingBlock,
    PromptCacheBreakpoint,
)


class AnthropicMessagesTextParam(TypedDict, total=False):
    type: Required[Literal["text"]]
    text: Required[str]
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None
    prompt_cache_breakpoint: ReadOnly[PromptCacheBreakpoint]


class AnthropicMessagesToolUseParam(TypedDict, total=False):
    type: Required[Literal["tool_use"]]
    id: str
    name: str
    input: dict[str, JsonValue]
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None
    caller: ToolCaller | None


class CompactionBlock(TypedDict, total=False):
    type: Required[ReadOnly[Literal["compaction"]]]
    content: ReadOnly[str | None]
    signature: ReadOnly[str]


# ast-grep-ignore: no-long-names
AnthropicMessagesAssistantMessageValues: TypeAlias = (
    AnthropicMessagesTextParam
    | AnthropicMessagesToolUseParam
    | ChatCompletionThinkingBlock
    # ast-grep-ignore: no-long-names
    | ChatCompletionRedactedThinkingBlock
    | CompactionBlock
)


# ast-grep-ignore: no-long-names
class AnthopicMessagesAssistantMessageParam(TypedDict, total=False):
    # ast-grep-ignore: no-long-names
    content: Required[str | Iterable[AnthropicMessagesAssistantMessageValues]]
    role: Required[Literal["assistant"]]
    name: str


class AnthropicContentParamSource(TypedDict):
    type: Literal["base64"]
    media_type: str
    data: str


class AnthropicContentParamSourceUrl(TypedDict):
    type: Literal["url"]
    url: str


# ast-grep-ignore: no-long-names
class AnthropicContentParamSourceFileId(TypedDict):
    type: Literal["file"]
    file_id: str


# ast-grep-ignore: no-long-names
class AnthropicContentParamSourceText(TypedDict):
    type: ReadOnly[Literal["text"]]
    media_type: ReadOnly[Literal["text/plain"]]
    data: ReadOnly[str]


# ast-grep-ignore: no-long-names
class AnthropicContentParamSourceContent(TypedDict):
    type: ReadOnly[Literal["content"]]
    content: ReadOnly[str | Sequence["AnthropicMessagesTextParam | AnthropicMessagesImageParam"]]


# ast-grep-ignore: no-long-names
class AnthropicMessagesContainerUploadParam(TypedDict, total=False):
    type: Required[Literal["container_upload"]]
    file_id: str
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None


class AnthropicMessagesImageParam(TypedDict, total=False):
    type: Required[Literal["image"]]
    source: Required[
        AnthropicContentParamSource
        # ast-grep-ignore: no-long-names
        | AnthropicContentParamSourceFileId
        | AnthropicContentParamSourceUrl
    ]
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None
    prompt_cache_breakpoint: ReadOnly[PromptCacheBreakpoint]


class CitationsObject(TypedDict):
    enabled: bool


class AnthropicCitationPageLocation(TypedDict, total=False):
    type: Literal["page_location"]
    cited_text: str
    document_index: int
    document_title: str | None
    start_page_number: int
    end_page_number: int


class AnthropicCitationCharLocation(TypedDict, total=False):
    type: Literal["char_location"]
    cited_text: str
    document_index: int
    document_title: str | None
    start_char_index: int
    end_char_index: int


AnthropicCitation: TypeAlias = AnthropicCitationPageLocation | AnthropicCitationCharLocation


class AnthropicMessagesDocumentParam(TypedDict, total=False):
    type: Required[Literal["document"]]
    source: Required[
        AnthropicContentParamSource
        # ast-grep-ignore: no-long-names
        | AnthropicContentParamSourceFileId
        | AnthropicContentParamSourceUrl
        # ast-grep-ignore: no-long-names
        | AnthropicContentParamSourceText
        # ast-grep-ignore: no-long-names
        | AnthropicContentParamSourceContent
    ]
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None
    title: str
    context: str
    citations: CitationsObject | None


# ast-grep-ignore: no-long-names
class AnthropicMessagesToolResultContent(TypedDict, total=False):
    type: Required[Literal["text"]]
    text: Required[str]
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None


# ast-grep-ignore: no-long-names
class AnthropicMessagesToolResultParam(TypedDict, total=False):
    type: Required[Literal["tool_result"]]
    tool_use_id: Required[str]
    is_error: bool
    content: (
        str
        | Iterable[
            # ast-grep-ignore: no-long-names
            AnthropicMessagesToolResultContent
            | AnthropicMessagesImageParam
            | AnthropicMessagesDocumentParam
            | ToolReference
        ]
    )
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None


# ast-grep-ignore: no-long-names
AnthropicMessagesUserMessageValues: TypeAlias = (
    AnthropicMessagesTextParam
    | AnthropicMessagesImageParam
    # ast-grep-ignore: no-long-names
    | AnthropicMessagesToolResultParam
    | AnthropicMessagesDocumentParam
    # ast-grep-ignore: no-long-names
    | AnthropicMessagesContainerUploadParam
)


# ast-grep-ignore: no-long-names
class AnthropicMessagesUserMessageParam(TypedDict, total=False):
    role: Required[Literal["user"]]
    # ast-grep-ignore: no-long-names
    content: Required[str | Iterable[AnthropicMessagesUserMessageValues]]


class AnthropicMetadata(TypedDict, total=False):
    user_id: str


class AnthropicSystemMessageContent(TypedDict, total=False):
    type: str
    text: str
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None
    prompt_cache_breakpoint: ReadOnly[PromptCacheBreakpoint]


# ast-grep-ignore: no-long-names
class AnthropicMessagesSystemMessageParam(TypedDict, total=False):
    role: Required[Literal["system"]]
    content: Required[str | Iterable[AnthropicSystemMessageContent]]


AllAnthropicMessageValues: TypeAlias = (
    # ast-grep-ignore: no-long-names
    AnthropicMessagesUserMessageParam | AnthopicMessagesAssistantMessageParam
)

# ast-grep-ignore: no-long-names
AllAnthropicPassThroughMessageValues: TypeAlias = (
    # ast-grep-ignore: no-long-names
    AnthropicMessagesUserMessageParam
    # ast-grep-ignore: no-long-names
    | AnthopicMessagesAssistantMessageParam
    # ast-grep-ignore: no-long-names
    | AnthropicMessagesSystemMessageParam
)


class AnthropicCompaction(TypedDict, total=False):
    type: Required[ReadOnly[Literal["summarize"]]]
    instructions: ReadOnly[str]
