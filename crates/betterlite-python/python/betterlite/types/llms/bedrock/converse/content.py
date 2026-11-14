from typing import Literal, Required, TypeAlias

from pydantic import JsonValue
from typing_extensions import ReadOnly, TypedDict


class CachePointBlock(TypedDict, total=False):
    type: Literal["default"]
    ttl: str


class SystemContentBlock(TypedDict, total=False):
    text: str
    cachePoint: CachePointBlock


class SourceBlock(TypedDict):
    bytes: str | None


BedrockImageTypes: TypeAlias = Literal["png", "jpeg", "gif", "webp"]


class ImageBlock(TypedDict):
    format: BedrockImageTypes | str
    source: SourceBlock


BedrockVideoTypes: TypeAlias = Literal[
    "mp4", "mov", "mkv", "webm", "flv", "mpeg", "mpg", "wmv", "3gp",
]


class VideoBlock(TypedDict):
    format: BedrockVideoTypes | str
    source: SourceBlock


BedrockDocumentTypes: TypeAlias = Literal[
    "pdf", "csv", "doc", "docx", "xls", "xlsx", "html", "txt", "md",
]


class DocumentBlock(TypedDict):
    format: BedrockDocumentTypes | str
    source: SourceBlock
    name: str


class SearchResultBlock(TypedDict, total=False):
    source: str
    title: str
    content: list[dict[str, JsonValue]]
    citations: dict[str, JsonValue]


class ToolResultContentBlock(TypedDict, total=False):
    image: ImageBlock
    document: DocumentBlock
    searchResult: SearchResultBlock
    json: dict[str, JsonValue]
    text: str


class ToolResultBlock(TypedDict, total=False):
    content: Required[list[ToolResultContentBlock]]
    toolUseId: Required[str]
    status: Literal["success", "error"]


class ToolUseBlock(TypedDict):
    input: dict[str, JsonValue]
    name: str
    toolUseId: str


class BedrockConverseReasoningTextBlock(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    text: Required[str]
    signature: str


# ast-grep-ignore: no-long-names
class BedrockConverseReasoningContentBlock(TypedDict, total=False):
    reasoningText: BedrockConverseReasoningTextBlock  # ast-grep-ignore: no-long-names
    redactedContent: str


# ast-grep-ignore: no-long-names
class BedrockConverseReasoningContentBlockDelta(TypedDict, total=False):
    signature: str
    redactedContent: str
    text: str


class BedrockConverseGptReasoningEffortBlock(TypedDict):  # ast-grep-ignore: no-long-names
    effort: ReadOnly[str]


class GuardrailConverseTextBlock(TypedDict, total=False):
    text: str


class GuardrailConverseContentBlock(TypedDict, total=False):
    text: GuardrailConverseTextBlock
