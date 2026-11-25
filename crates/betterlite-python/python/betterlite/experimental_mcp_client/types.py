from typing import ClassVar, Final, Literal, TypeAlias

from pydantic import ConfigDict, Field, JsonValue, TypeAdapter

from betterlite.types.utils.base import ResponseObject


# The MCP objects LiteLLM hands back: snake_case attributes, wire names on a dump
class McpResult(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(populate_by_name=True)


class TextContent(McpResult):
    type: Literal["text"] = "text"
    text: str


class ImageContent(McpResult):
    type: Literal["image"] = "image"
    data: str
    mime_type: str = Field(alias="mimeType")


class AudioContent(McpResult):
    type: Literal["audio"] = "audio"
    data: str
    mime_type: str = Field(alias="mimeType")


class TextResourceContents(McpResult):
    uri: str
    mime_type: str | None = Field(default=None, alias="mimeType")
    text: str


class BlobResourceContents(McpResult):
    uri: str
    mime_type: str | None = Field(default=None, alias="mimeType")
    blob: str


ResourceContent: TypeAlias = TextResourceContents | BlobResourceContents


class ResourceLink(McpResult):
    type: Literal["resource_link"] = "resource_link"
    uri: str
    name: str
    title: str | None = None
    description: str | None = None
    mime_type: str | None = Field(default=None, alias="mimeType")
    size: int | None = None


class EmbeddedResource(McpResult):
    type: Literal["resource"] = "resource"
    resource: ResourceContent


ContentBlock: TypeAlias = (
    TextContent | ImageContent | AudioContent | ResourceLink | EmbeddedResource
)


class CallToolResult(McpResult):
    content: list[ContentBlock]
    is_error: bool = Field(default=False, alias="isError")
    structured_content: JsonValue = Field(default=None, alias="structuredContent")
    result_type: Literal["complete"] = Field(default="complete", alias="resultType")


class InputRequiredResult(McpResult):
    input_requests: dict[str, JsonValue] | None = Field(default=None, alias="inputRequests")
    request_state: str | None = Field(default=None, alias="requestState")
    result_type: Literal["input_required"] = Field(
        default="input_required", alias="resultType"
    )


ToolCallResult: TypeAlias = CallToolResult | InputRequiredResult


class Tool(McpResult):
    name: str
    description: str | None = None
    input_schema: dict[str, JsonValue] = Field(alias="inputSchema")


class ListToolsResult(McpResult):
    tools: list[Tool]
    next_cursor: str | None = Field(default=None, alias="nextCursor")
    ttl_ms: int = Field(default=0, alias="ttlMs")
    cache_scope: Literal["public", "private"] = Field(default="private", alias="cacheScope")
    result_type: Literal["complete"] = Field(default="complete", alias="resultType")


class PromptArgument(McpResult):
    name: str
    description: str | None = None
    required: bool | None = None


class Prompt(McpResult):
    name: str
    description: str | None = None
    arguments: list[PromptArgument] | None = None


class PromptMessage(McpResult):
    role: Literal["user", "assistant"]
    content: ContentBlock


class GetPromptResult(McpResult):
    description: str | None = None
    messages: list[PromptMessage]
    result_type: Literal["complete"] = Field(default="complete", alias="resultType")


class Resource(McpResult):
    uri: str
    name: str
    title: str | None = None
    description: str | None = None
    mime_type: str | None = Field(default=None, alias="mimeType")
    size: int | None = None


class ResourceTemplate(McpResult):
    uri_template: str = Field(alias="uriTemplate")
    name: str
    title: str | None = None
    description: str | None = None
    mime_type: str | None = Field(default=None, alias="mimeType")


class ReadResourceResult(McpResult):
    contents: list[ResourceContent]
    ttl_ms: int = Field(default=0, alias="ttlMs")
    cache_scope: Literal["public", "private"] = Field(default="private", alias="cacheScope")
    result_type: Literal["complete"] = Field(default="complete", alias="resultType")


class CallToolRequestParams(McpResult):
    name: str
    arguments: dict[str, JsonValue] | None = None


class GetPromptRequestParams(McpResult):
    name: str
    arguments: dict[str, str] | None = None


class PaginatedRequestParams(McpResult):
    cursor: str | None = None


TOOL_PAGE: Final[TypeAdapter[ListToolsResult]] = TypeAdapter(ListToolsResult)
TOOL_LIST: Final[TypeAdapter[list[Tool]]] = TypeAdapter(list[Tool])
CALL_RESULT: Final[TypeAdapter[CallToolResult]] = TypeAdapter(CallToolResult)
TOOL_CALL: Final[TypeAdapter[ToolCallResult]] = TypeAdapter(ToolCallResult)
PROMPT_LIST: Final[TypeAdapter[list[Prompt]]] = TypeAdapter(list[Prompt])
PROMPT_RESULT: Final[TypeAdapter[GetPromptResult]] = TypeAdapter(GetPromptResult)
RESOURCE_LIST: Final[TypeAdapter[list[Resource]]] = TypeAdapter(list[Resource])
TEMPLATE_LIST: Final[TypeAdapter[list[ResourceTemplate]]] = TypeAdapter(list[ResourceTemplate])
READ_RESULT: Final[TypeAdapter[ReadResourceResult]] = TypeAdapter(ReadResourceResult)
