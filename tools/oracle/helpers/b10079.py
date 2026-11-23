from typing import Literal, NotRequired, TypedDict

import litellm
from litellm.proxy._experimental.mcp_server.mcp_server_manager import MCPServerManager
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import JsonValue


class RegistryCall(TypedDict):
    config_servers: list[dict[str, JsonValue]]
    db_servers: list[dict[str, JsonValue]]
    public_mcp_servers: list[str] | None
    public_hub_strict: bool


class ListCall(RegistryCall):
    function: Literal["list"]


class IsPublicCall(RegistryCall):
    function: Literal["is_public"]
    server_id: str
    public_ids: NotRequired[list[str] | None]


Input = ListCall | IsPublicCall


def servers_by_id(records: list[dict[str, JsonValue]]) -> dict[str, MCPServer]:
    servers: dict[str, MCPServer] = {}
    for record in records:
        server = MCPServer.model_validate(record)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/mcp_server/mcp_server_manager.py#L88
        servers[server.server_id] = server
    return servers


# MCPServerManager.__init__ builds token stores and discovery caches the public methods never read
def build_manager(call: RegistryCall) -> MCPServerManager:
    manager = MCPServerManager.__new__(MCPServerManager)
    manager.config_mcp_servers = servers_by_id(call["config_servers"])
    manager.registry = servers_by_id(call["db_servers"])
    return manager


def list_servers(call: ListCall) -> list[str]:
    manager = build_manager(call)
    return [server.server_id for server in manager.get_public_mcp_servers()]


def is_public(call: IsPublicCall) -> bool:
    manager = build_manager(call)
    return manager.is_mcp_server_public(call["server_id"], public_ids=call.get("public_ids"))


def answer(input_value: Input) -> list[str] | bool:
    match input_value["function"]:
        case "list":
            return list_servers(input_value)
        case "is_public":
            return is_public(input_value)


def run(input_value: Input) -> list[str] | bool:
    saved_servers = litellm.public_mcp_servers
    saved_strict = litellm.public_mcp_hub_strict_whitelist
    try:
        litellm.public_mcp_servers = input_value["public_mcp_servers"]
        litellm.public_mcp_hub_strict_whitelist = input_value["public_hub_strict"]
        return answer(input_value)
    finally:
        litellm.public_mcp_servers = saved_servers
        litellm.public_mcp_hub_strict_whitelist = saved_strict
