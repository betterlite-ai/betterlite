from collections.abc import Iterable
from typing import Literal, Required, TypeAlias, TypedDict

from pydantic import JsonValue
from typing_extensions import ReadOnly


class ChatCompletionAudioDelta(TypedDict, total=False):
    data: str
    transcript: str
    expires_at: int
    id: str


# ast-grep-ignore: no-long-names
class ChatCompletionToolCallFunctionChunk(TypedDict, total=False):
    name: str | None
    arguments: str
    provider_specific_fields: dict[str, JsonValue] | None


# ast-grep-ignore: no-long-names
class ChatCompletionAssistantToolCall(TypedDict):
    id: str | None
    type: Literal["function"]
    function: ChatCompletionToolCallFunctionChunk  # ast-grep-ignore: no-long-names


class ChatCompletionToolCallChunk(TypedDict):
    id: str | None
    type: Literal["function"]
    function: ChatCompletionToolCallFunctionChunk  # ast-grep-ignore: no-long-names
    index: int


# ast-grep-ignore: no-long-names
class ChatCompletionDeltaToolCallChunk(TypedDict, total=False):
    id: str
    type: Literal["function"]
    function: ChatCompletionToolCallFunctionChunk  # ast-grep-ignore: no-long-names
    index: int


class ChatCompletionCachedContent(TypedDict, total=False):
    type: Required[Literal["ephemeral"]]
    ttl: Literal["5m", "1h"]


class PromptCacheBreakpoint(TypedDict):
    mode: ReadOnly[Literal["explicit"]]


class PromptCacheOptions(TypedDict, total=False):
    mode: ReadOnly[Literal["implicit", "explicit"]]
    ttl: ReadOnly[Literal["30m"]]


class ChatCompletionThinkingBlock(TypedDict, total=False):
    type: Required[Literal["thinking"]]
    thinking: str
    signature: str | None
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None


# ast-grep-ignore: no-long-names
class ChatCompletionRedactedThinkingBlock(TypedDict, total=False):
    type: Required[Literal["redacted_thinking"]]
    data: str
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None


# ast-grep-ignore: no-long-names
class ChatCompletionReasoningSummaryTextBlock(TypedDict, total=False):
    type: Required[Literal["summary_text"]]
    text: str


class ChatCompletionReasoningItem(TypedDict, total=False):
    type: Required[Literal["reasoning"]]
    id: str
    encrypted_content: str | None
    # ast-grep-ignore: no-long-names
    summary: ReadOnly[list[ChatCompletionReasoningSummaryTextBlock]]


# ast-grep-ignore: no-long-names
class WebSearchOptionsUserLocationApproximate(TypedDict, total=False):
    city: str
    country: str
    region: str
    timezone: str


class WebSearchOptionsUserLocation(TypedDict, total=False):
    # ast-grep-ignore: no-long-names
    approximate: Required[WebSearchOptionsUserLocationApproximate]
    type: Required[Literal["approximate"]]


class WebSearchOptions(TypedDict, total=False):
    search_context_size: Literal["low", "medium", "high"]
    user_location: WebSearchOptionsUserLocation | None


class FileSearchTool(TypedDict, total=False):
    type: Literal["file_search"]
    vector_store_ids: list[str] | None


# ast-grep-ignore: no-long-names
class ChatCompletionAnnotationURLCitation(TypedDict, total=False):
    end_index: int
    start_index: int
    title: str
    url: str


class ChatCompletionAnnotation(TypedDict, total=False):
    type: Literal["url_citation"]
    url_citation: ChatCompletionAnnotationURLCitation  # ast-grep-ignore: no-long-names


class OpenAIChatCompletionTextObject(TypedDict):
    type: Literal["text"]
    text: str


class ChatCompletionTextObject(OpenAIChatCompletionTextObject, total=False):
    cache_control: ChatCompletionCachedContent


class ChatCompletionImageUrlObject(TypedDict, total=False):
    url: Required[str]
    detail: str
    format: str


class ChatCompletionImageObject(TypedDict):
    type: Literal["image_url"]
    image_url: str | ChatCompletionImageUrlObject


class ChatCompletionVideoUrlObject(TypedDict, total=False):
    url: Required[str]
    detail: str


class ChatCompletionVideoObject(TypedDict):
    type: Literal["video_url"]
    video_url: str | ChatCompletionVideoUrlObject


class _InputAudio(TypedDict, total=False):
    data: Required[str]
    format: Required[Literal["wav", "mp3"]]


# the pinned openai adds prompt_cache_breakpoint to the SDK's input-audio TypedDict
class ChatCompletionAudioObject(TypedDict, total=False):
    input_audio: Required[_InputAudio]
    type: Required[Literal["input_audio"]]
    prompt_cache_breakpoint: PromptCacheBreakpoint


class DocumentObject(TypedDict):
    type: Literal["text"]
    media_type: str
    data: str


class CitationsObject(TypedDict):
    enabled: bool


class ChatCompletionDocumentObject(TypedDict):
    type: Literal["document"]
    source: DocumentObject
    title: str
    context: str
    citations: CitationsObject | None


class ChatCompletionFileObjectFile(TypedDict, total=False):
    file_data: str
    file_id: str
    filename: str
    format: str
    detail: str
    video_metadata: dict[str, JsonValue]


class ChatCompletionFileObject(TypedDict):
    type: Literal["file"]
    file: ChatCompletionFileObjectFile


OpenAIMessageContentListBlock: TypeAlias = (
    ChatCompletionTextObject
    | ChatCompletionImageObject
    | ChatCompletionAudioObject
    | ChatCompletionDocumentObject
    | ChatCompletionVideoObject
    | ChatCompletionFileObject
)

OpenAIMessageContent: TypeAlias = str | Iterable[OpenAIMessageContentListBlock]
