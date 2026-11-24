import asyncio
import os
import sys
from collections.abc import Callable, Coroutine, Mapping, Sequence
from datetime import UTC, datetime
from importlib import import_module
from types import ModuleType
from typing import Final, Literal, Protocol, TypedDict
from urllib.parse import urlsplit

import litellm
from litellm.proxy._experimental.mcp_server.mcp_server_manager import (
    global_mcp_server_manager,
)
from litellm.proxy.agent_endpoints.agent_registry import global_agent_registry
from litellm.proxy.public_endpoints.public_endpoints import (
    get_agents,
    get_mcp_servers,
    public_skill_hub,
)
from litellm.types.agents import AgentCard, AgentResponse
from litellm.types.mcp import MCPPublicServer
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from litellm.types.proxy.claude_code_endpoints import ListPluginsResponse
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Scope

PROXY_SERVER_MODULE: Final = "litellm.proxy.proxy_server"
MARKETPLACE_MODULE: Final = (
    "litellm.proxy.anthropic_endpoints.claude_code_endpoints.claude_code_marketplace"
)
PROXY_BASE_URL_ENV: Final = "PROXY_BASE_URL"
ROOT_PATH_ENV: Final = "SERVER_ROOT_PATH"


class AgentHubCall(TypedDict):
    function: Literal["agent_hub"]
    agents: list[dict[str, JsonValue]]
    public_agent_groups: list[str] | None
    request_base_url: str
    root_path: str
    proxy_base_url: str | None


class McpHubCall(TypedDict):
    function: Literal["mcp_hub"]
    config_servers: list[dict[str, JsonValue]]
    db_servers: list[dict[str, JsonValue]]
    public_mcp_servers: list[str] | None
    public_hub_strict: bool


class PluginRowInput(TypedDict):
    id: str
    name: str
    manifest_json: str | None
    enabled: bool
    created_at: str | None
    updated_at: str | None


class SkillHubCall(TypedDict):
    function: Literal["skill_hub"]
    plugins: list[PluginRowInput]
    table_error: bool


Call = AgentHubCall | McpHubCall | SkillHubCall


class PluginAuthorJson(TypedDict):
    name: str
    email: str | None


class PluginItemJson(TypedDict):
    id: str
    name: str
    version: str | None
    description: str | None
    source: dict[str, str]
    author: PluginAuthorJson | None
    homepage: str | None
    keywords: list[str] | None
    category: str | None
    domain: str | None
    namespace: str | None
    enabled: bool
    created_at: str | None
    updated_at: str | None


class PluginListJson(TypedDict):
    plugins: list[PluginItemJson]
    count: int


class McpHubRoute(Protocol):
    async def __call__(self) -> list[MCPPublicServer]: ...


class SkillHubRoute(Protocol):
    async def __call__(self) -> ListPluginsResponse: ...


# LiteLLM infers the agent route's return type, so this name pins the JSON its call returns
AGENT_HUB: Final[
    Callable[[Request], Coroutine[JsonValue, JsonValue, list[dict[str, JsonValue]]]]
] = get_agents
MCP_HUB: Final[McpHubRoute] = get_mcp_servers
SKILL_HUB: Final[SkillHubRoute] = public_skill_hub
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
AGENT_CARDS: Final[TypeAdapter[list[AgentCard]]] = TypeAdapter(list[AgentCard])
AGENT_RESPONSE: Final[TypeAdapter[AgentResponse]] = TypeAdapter(AgentResponse)
MCP_SERVER: Final[TypeAdapter[MCPServer]] = TypeAdapter(MCPServer)
MCP_PUBLIC_SERVERS: Final[TypeAdapter[list[MCPPublicServer]]] = TypeAdapter(
    list[MCPPublicServer]
)
PLUGIN_LIST: Final[TypeAdapter[PluginListJson]] = TypeAdapter(PluginListJson)


class PluginRow:
    def __init__(self, record: PluginRowInput) -> None:
        self.id = record["id"]
        self.name = record["name"]
        self.enabled = record["enabled"]
        self.manifest_json = record["manifest_json"]
        self.created_at = utc_timestamp(record["created_at"])
        self.updated_at = utc_timestamp(record["updated_at"])


class PluginActions:
    def __init__(self, rows: Sequence[PluginRow], *, fails: bool) -> None:
        self.rows = rows
        self.fails = fails

    async def find_many(self, *, where: Mapping[str, bool]) -> Sequence[PluginRow]:
        if where != {"enabled": True}:
            raise RuntimeError(f"the plugin query changed: {where}")
        await asyncio.sleep(0)
        if self.fails:
            raise RuntimeError("the plugin table failed")
        return self.rows


class PluginDatabase:
    def __init__(self, actions: PluginActions) -> None:
        self.litellm_claudecodeplugintable = actions


class PluginPrismaClient:
    def __init__(self, database: PluginDatabase) -> None:
        self.db = database


class ProxyServerModule(ModuleType):
    prisma_client: PluginPrismaClient | None = None


def utc_timestamp(value: str | None) -> datetime | None:
    if value is None:
        return None
    parsed = datetime.fromisoformat(value)
    if parsed.tzinfo is None:
        return parsed.replace(tzinfo=UTC)
    return parsed


def install_module(name: str, module: ModuleType) -> ModuleType | None:
    saved = sys.modules.get(name)
    sys.modules[name] = module
    return saved


def restore_module(name: str, saved: ModuleType | None) -> None:
    if saved is None:
        del sys.modules[name]
        return
    sys.modules[name] = saved


def apply_env(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
        return
    os.environ[name] = value


def servers_by_id(records: list[dict[str, JsonValue]]) -> dict[str, MCPServer]:
    servers: dict[str, MCPServer] = {}
    for record in records:
        server = MCP_SERVER.validate_python(record)
        servers[server.server_id] = server
    return servers


def build_request(base_url: str, root_path: str) -> Request:
    parts = urlsplit(base_url)
    headers: list[tuple[bytes, bytes]] = [(b"host", parts.netloc.encode("latin-1"))]
    scope: Scope = {
        "type": "http",
        "method": "GET",
        "scheme": parts.scheme,
        "path": "/public/agent_hub",
        "query_string": b"",
        "root_path": root_path,
        "headers": headers,
    }
    return Request(scope)


def apply_agent_hub(call: AgentHubCall) -> None:
    global_agent_registry.agent_list = [
        AGENT_RESPONSE.validate_python(record) for record in call["agents"]
    ]
    litellm.public_agent_groups = call["public_agent_groups"]
    apply_env(PROXY_BASE_URL_ENV, call["proxy_base_url"])
    os.environ[ROOT_PATH_ENV] = call["root_path"]


def apply_mcp_hub(call: McpHubCall) -> None:
    global_mcp_server_manager.config_mcp_servers = servers_by_id(call["config_servers"])
    global_mcp_server_manager.registry = servers_by_id(call["db_servers"])
    litellm.public_mcp_servers = call["public_mcp_servers"]
    litellm.public_mcp_hub_strict_whitelist = call["public_hub_strict"]


async def hub_answer(call: AgentHubCall | McpHubCall) -> JsonValue:
    saved_agents = global_agent_registry.agent_list
    saved_groups = litellm.public_agent_groups
    saved_config = global_mcp_server_manager.config_mcp_servers
    saved_registry = global_mcp_server_manager.registry
    saved_servers = litellm.public_mcp_servers
    saved_strict = litellm.public_mcp_hub_strict_whitelist
    saved_proxy = os.environ.get(PROXY_BASE_URL_ENV)
    saved_root = os.environ.get(ROOT_PATH_ENV)
    try:
        match call["function"]:
            case "agent_hub":
                apply_agent_hub(call)
                request = build_request(call["request_base_url"], call["root_path"])
                cards = AGENT_CARDS.validate_python(await AGENT_HUB(request))
                return JSON.validate_python(cards)
            case "mcp_hub":
                apply_mcp_hub(call)
                servers = await MCP_HUB()
                return JSON.validate_json(MCP_PUBLIC_SERVERS.dump_json(servers))
    finally:
        global_agent_registry.agent_list = saved_agents
        litellm.public_agent_groups = saved_groups
        global_mcp_server_manager.config_mcp_servers = saved_config
        global_mcp_server_manager.registry = saved_registry
        litellm.public_mcp_servers = saved_servers
        litellm.public_mcp_hub_strict_whitelist = saved_strict
        apply_env(PROXY_BASE_URL_ENV, saved_proxy)
        apply_env(ROOT_PATH_ENV, saved_root)


async def skill_hub_answer(call: SkillHubCall) -> JsonValue:
    import_module(MARKETPLACE_MODULE)
    rows = [PluginRow(record) for record in call["plugins"]]
    actions = PluginActions(rows, fails=call["table_error"])
    database = PluginDatabase(actions)
    client = PluginPrismaClient(database)
    module = ProxyServerModule(PROXY_SERVER_MODULE)
    module.prisma_client = client
    saved_module = install_module(PROXY_SERVER_MODULE, module)
    try:
        response = await SKILL_HUB()
    finally:
        restore_module(PROXY_SERVER_MODULE, saved_module)
    return JSON.validate_python(PLUGIN_LIST.validate_json(response.model_dump_json()))


def run(input_value: Call) -> JsonValue:
    match input_value["function"]:
        case "agent_hub":
            return asyncio.run(hub_answer(input_value))
        case "mcp_hub":
            return asyncio.run(hub_answer(input_value))
        case "skill_hub":
            return asyncio.run(skill_hub_answer(input_value))
