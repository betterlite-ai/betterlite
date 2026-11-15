from typing import Literal

from pydantic import Base64Str, BaseModel, Field, JsonValue

from betterlite.types.interactions.content import (
    Annotation,
    CodeExecutionCallArguments,
    FileSearchResult,
    GoogleSearchCallArguments,
    GoogleSearchResult,
    ImageContent,
    Result,
    TextContent,
    UrlContextCallArguments,
    UrlContextResult,
)
from betterlite.types.interactions.enums import MediaResolution
from betterlite.types.interactions.options import (
    AudioMimeTypeOption,
    ImageMimeTypeOption,
    VideoMimeTypeOption,
)


class TextDelta(BaseModel):
    text: str | None = None
    type: Literal["text"]
    annotations: list[Annotation] | None = None


class DocumentDelta(BaseModel):
    data: Base64Str | None = None
    uri: str | None = None
    mime_type: str | None = None
    type: Literal["document"]


class ThoughtSignatureDelta(BaseModel):
    signature: Base64Str | None = None
    type: Literal["thought_signature"]


class FunctionCallDelta(BaseModel):
    name: str | None = None
    arguments: dict[str, JsonValue] | None = None
    type: Literal["function_call"]
    id: str | None = None


class CodeExecutionCallDelta(BaseModel):
    arguments: CodeExecutionCallArguments | None = None
    type: Literal["code_execution_call"]
    id: str | None = None


class UrlContextCallDelta(BaseModel):
    arguments: UrlContextCallArguments | None = None
    type: Literal["url_context_call"]
    id: str | None = None


class GoogleSearchCallDelta(BaseModel):
    arguments: GoogleSearchCallArguments | None = None
    type: Literal["google_search_call"]
    id: str | None = None


class McpServerToolCallDelta(BaseModel):
    name: str | None = None
    server_name: str | None = None
    arguments: dict[str, JsonValue] | None = None
    type: Literal["mcp_server_tool_call"]
    id: str | None = None


class CodeExecutionResultDelta(BaseModel):
    result: str | None = None
    is_error: bool | None = None
    signature: str | None = None
    type: Literal["code_execution_result"]
    call_id: str | None = None


class UrlContextResultDelta(BaseModel):
    signature: str | None = None
    result: list[UrlContextResult] | None = None
    is_error: bool | None = None
    type: Literal["url_context_result"]
    call_id: str | None = None


class GoogleSearchResultDelta(BaseModel):
    signature: str | None = None
    result: list[GoogleSearchResult] | None = None
    is_error: bool | None = None
    type: Literal["google_search_result"]
    call_id: str | None = None


class FileSearchResultDelta(BaseModel):
    result: list[FileSearchResult] | None = None
    type: Literal["file_search_result"]


class ImageDelta(BaseModel):
    data: Base64Str | None = None
    uri: str | None = None
    mime_type: ImageMimeTypeOption | None = None
    type: Literal["image"]
    resolution: MediaResolution | None = None


class AudioDelta(BaseModel):
    data: Base64Str | None = None
    uri: str | None = None
    mime_type: AudioMimeTypeOption | None = None
    type: Literal["audio"]


class VideoDelta(BaseModel):
    data: Base64Str | None = None
    uri: str | None = None
    mime_type: VideoMimeTypeOption | None = None
    type: Literal["video"]
    resolution: MediaResolution | None = None


class ThoughtSummaryDelta(BaseModel):
    type: Literal["thought_summary"]
    content: TextContent | ImageContent | None = Field(default=None, discriminator="type")


class FunctionResultDelta(BaseModel):
    name: str | None = None
    is_error: bool | None = None
    type: Literal["function_result"]
    result: Result | str | None = None
    call_id: str | None = None


class McpServerToolResultDelta(BaseModel):
    name: str | None = None
    server_name: str | None = None
    type: Literal["mcp_server_tool_result"]
    result: Result | str | None = None
    call_id: str | None = None


class ContentDelta(BaseModel):
    index: int | None = None
    event_type: Literal["content.delta"] = "content.delta"
    event_id: str | None = None
    delta: (
        TextDelta
        | ImageDelta
        | AudioDelta
        | DocumentDelta
        | VideoDelta
        | ThoughtSummaryDelta
        | ThoughtSignatureDelta
        | FunctionCallDelta
        | FunctionResultDelta
        | CodeExecutionCallDelta
        | CodeExecutionResultDelta
        | UrlContextCallDelta
        | UrlContextResultDelta
        | GoogleSearchCallDelta
        | GoogleSearchResultDelta
        | McpServerToolCallDelta
        | McpServerToolResultDelta
        | FileSearchResultDelta
        | None
    ) = Field(default=None, discriminator="type")
