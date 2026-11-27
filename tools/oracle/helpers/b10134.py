from typing import Final, TypedDict

import litellm
from litellm.proxy._experimental.mcp_server.mcp_server_manager import (
    global_mcp_server_manager,
)
from litellm.proxy._types import LiteLLM_MCPServerTable
from litellm.proxy.management_endpoints.mcp_management_endpoints import _redact_mcp_credentials  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/mcp_management_endpoints.py#L639
from pydantic import JsonValue, TypeAdapter

from b10079 import servers_by_id


class Input(TypedDict):
    server: dict[str, JsonValue]
    config_servers: list[dict[str, JsonValue]]
    db_servers: list[dict[str, JsonValue]]
    public_mcp_servers: list[str] | None
    public_hub_strict: bool


class Answer(TypedDict):
    credentials: JsonValue
    mcp_info: JsonValue


ANSWER: Final[TypeAdapter[Answer]] = TypeAdapter(Answer)


def run(input_value: Input) -> Answer:
    # LiteLLM types the manager global as Final, so its two server maps are swapped in place
    saved_config = global_mcp_server_manager.config_mcp_servers
    saved_registry = global_mcp_server_manager.registry
    saved_servers = litellm.public_mcp_servers
    saved_strict = litellm.public_mcp_hub_strict_whitelist
    try:
        global_mcp_server_manager.config_mcp_servers = servers_by_id(
            input_value["config_servers"]
        )
        global_mcp_server_manager.registry = servers_by_id(input_value["db_servers"])
        litellm.public_mcp_servers = input_value["public_mcp_servers"]
        litellm.public_mcp_hub_strict_whitelist = input_value["public_hub_strict"]
        server = LiteLLM_MCPServerTable.model_validate(input_value["server"])  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/mcp_server.py#L49
        return ANSWER.validate_json(_redact_mcp_credentials(server).model_dump_json())
    finally:
        global_mcp_server_manager.config_mcp_servers = saved_config
        global_mcp_server_manager.registry = saved_registry
        litellm.public_mcp_servers = saved_servers
        litellm.public_mcp_hub_strict_whitelist = saved_strict
