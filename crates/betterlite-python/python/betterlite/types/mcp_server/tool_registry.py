from collections.abc import Awaitable
from typing import ClassVar, Protocol, runtime_checkable

from pydantic import BaseModel, ConfigDict, JsonValue


@runtime_checkable
class ToolHandler(Protocol):
    def __call__(self, **arguments: JsonValue) -> JsonValue | Awaitable[JsonValue]: ...


class MCPTool(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(arbitrary_types_allowed=True)
    name: str
    description: str
    input_schema: dict[str, JsonValue]
    handler: ToolHandler


class ToolSchema(BaseModel):
    name: str
    description: str
    inputSchema: dict[str, JsonValue]  # noqa: N815 # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/mcp_server/tool_registry.py#L17


class ListToolsResponse(BaseModel):
    tools: list[ToolSchema]
    nextCursor: str | None = None  # noqa: N815 # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/mcp_server/tool_registry.py#L22
    _meta: dict[str, JsonValue] | None = None


class CallToolRequest(BaseModel):
    method: str = "tools/call"
    params: dict[str, JsonValue]


class ContentItem(BaseModel):
    type: str
    text: str | None = None
