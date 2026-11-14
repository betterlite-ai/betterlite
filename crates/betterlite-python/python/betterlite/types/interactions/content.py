from typing import Literal

from pydantic import Base64Str, BaseModel, Field, JsonValue, RootModel

from betterlite.types.interactions.enums import Language, MediaResolution, Status
from betterlite.types.interactions.options import (
    AudioMimeTypeOption,
    ImageMimeTypeOption,
    VideoMimeTypeOption,
)


class Annotation(BaseModel):
    start_index: int | None = None
    end_index: int | None = None
    source: str | None = None


class TextContent(BaseModel):
    text: str | None = None
    type: Literal["text"]
    annotations: list[Annotation] | None = None


class ImageContent(BaseModel):
    data: Base64Str | None = None
    uri: str | None = None
    mime_type: ImageMimeTypeOption | None = None
    type: Literal["image"]
    resolution: MediaResolution | None = None


class AudioContent(BaseModel):
    data: Base64Str | None = None
    uri: str | None = None
    mime_type: AudioMimeTypeOption | None = None
    type: Literal["audio"]


class DocumentContent(BaseModel):
    data: Base64Str | None = None
    uri: str | None = None
    mime_type: str | None = None
    type: Literal["document"]


class VideoContent(BaseModel):
    data: Base64Str | None = None
    uri: str | None = None
    mime_type: VideoMimeTypeOption | None = None
    type: Literal["video"]
    resolution: MediaResolution | None = None


class ThoughtSummary1(RootModel[TextContent | ImageContent]):
    root: TextContent | ImageContent = Field(discriminator="type")


class ThoughtSummary(RootModel[list[ThoughtSummary1]]):
    root: list[ThoughtSummary1]


class ThoughtContent(BaseModel):
    signature: Base64Str | None = None
    type: Literal["thought"]
    summary: ThoughtSummary | None = None


class FunctionCallContent(BaseModel):
    name: str
    arguments: dict[str, JsonValue]
    type: Literal["function_call"]
    id: str


class CodeExecutionCallArguments(BaseModel):
    language: Language | None = None
    code: str | None = None


class CodeExecutionCallContent(BaseModel):
    arguments: CodeExecutionCallArguments | None = None
    type: Literal["code_execution_call"]
    id: str | None = None


class CodeExecutionResultContent(BaseModel):
    result: str | None = None
    is_error: bool | None = None
    signature: str | None = None
    type: Literal["code_execution_result"]
    call_id: str | None = None


class UrlContextCallArguments(BaseModel):
    urls: list[str] | None = None


class UrlContextCallContent(BaseModel):
    arguments: UrlContextCallArguments | None = None
    type: Literal["url_context_call"]
    id: str | None = None


class UrlContextResult(BaseModel):
    url: str | None = None
    status: Status | None = None


class UrlContextResultContent(BaseModel):
    signature: str | None = None
    result: list[UrlContextResult] | None = None
    is_error: bool | None = None
    type: Literal["url_context_result"]
    call_id: str | None = None


class GoogleSearchCallArguments(BaseModel):
    queries: list[str] | None = None


class GoogleSearchCallContent(BaseModel):
    arguments: GoogleSearchCallArguments | None = None
    type: Literal["google_search_call"]
    id: str | None = None


class GoogleSearchResult(BaseModel):
    url: str | None = None
    title: str | None = None
    rendered_content: str | None = None


class GoogleSearchResultContent(BaseModel):
    signature: str | None = None
    result: list[GoogleSearchResult] | None = None
    is_error: bool | None = None
    type: Literal["google_search_result"]
    call_id: str | None = None


class McpServerToolCallContent(BaseModel):
    name: str
    server_name: str
    arguments: dict[str, JsonValue]
    type: Literal["mcp_server_tool_call"]
    id: str


class Result(BaseModel):
    items: list[str | ImageContent] | None = None


class FunctionResultContent(BaseModel):
    name: str | None = None
    is_error: bool | None = None
    type: Literal["function_result"]
    result: Result | dict[str, JsonValue] | str
    call_id: str


class McpServerToolResultContent(BaseModel):
    name: str | None = None
    server_name: str | None = None
    type: Literal["mcp_server_tool_result"]
    result: Result | dict[str, JsonValue] | str
    call_id: str


class FileSearchResult(BaseModel):
    title: str | None = None
    text: str | None = None
    file_search_store: str | None = None


class FileSearchResultContent(BaseModel):
    result: list[FileSearchResult] | None = None
    type: Literal["file_search_result"]


class Content(
    RootModel[
        TextContent
        | ImageContent
        | AudioContent
        | DocumentContent
        | VideoContent
        | ThoughtContent
        | FunctionCallContent
        | FunctionResultContent
        | CodeExecutionCallContent
        | CodeExecutionResultContent
        | UrlContextCallContent
        | UrlContextResultContent
        | GoogleSearchCallContent
        | GoogleSearchResultContent
        | McpServerToolCallContent
        | McpServerToolResultContent
        | FileSearchResultContent
    ]
):
    root: (
        TextContent
        | ImageContent
        | AudioContent
        | DocumentContent
        | VideoContent
        | ThoughtContent
        | FunctionCallContent
        | FunctionResultContent
        | CodeExecutionCallContent
        | CodeExecutionResultContent
        | UrlContextCallContent
        | UrlContextResultContent
        | GoogleSearchCallContent
        | GoogleSearchResultContent
        | McpServerToolCallContent
        | McpServerToolResultContent
        | FileSearchResultContent
    ) = Field(discriminator="type")
