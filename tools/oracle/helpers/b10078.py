from typing import Final, TypedDict

from litellm.proxy._experimental.mcp_server.auth.user_api_key_auth_mcp import MCPRequestHandler
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import TypeAdapter


class ServerRecord(TypedDict):
    server_id: str
    name: str
    transport: str
    access_groups: list[str] | None


class Input(TypedDict):
    servers: list[ServerRecord]
    access_groups: list[str]


SERVERS: Final[TypeAdapter[list[MCPServer]]] = TypeAdapter(list[MCPServer])


def run(input_value: Input) -> list[str]:
    servers = SERVERS.validate_python(input_value["servers"])
    by_server_id = {server.server_id: server for server in servers}
    found = MCPRequestHandler._get_config_server_ids_for_access_groups(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/auth/user_api_key_auth_mcp.py#L3595
        by_server_id, input_value["access_groups"]
    )
    return sorted(found)
