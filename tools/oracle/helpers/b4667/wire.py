from typing import Final

from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import JsonValue, TypeAdapter

UPSTREAM_URL: Final = "https://upstream.example.com/mcp"
JSON_ADAPTER: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
SERVER_ADAPTER: Final[TypeAdapter[MCPServer]] = TypeAdapter(MCPServer)
