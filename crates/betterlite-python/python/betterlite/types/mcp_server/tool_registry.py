from collections.abc import Awaitable, Callable
from typing import ClassVar

from pydantic import BaseModel, ConfigDict, JsonValue


class MCPTool(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(arbitrary_types_allowed=True)
    name: str
    description: str
    input_schema: dict[str, JsonValue]
    handler: Callable[..., JsonValue | Awaitable[JsonValue]]


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
