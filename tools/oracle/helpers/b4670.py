from typing import TypedDict

from litellm.proxy._experimental.mcp_server.auth.user_api_key_auth_mcp import _agent_capped_servers  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/auth/user_api_key_auth_mcp.py#L208-L220


class Input(TypedDict):
    allowed: list[str]
    agent_servers: list[str]
    access_group_servers: list[str] | None


def run(input_value: Input) -> list[str] | None:
    access_group_servers = input_value["access_group_servers"]
    ceiling = None if access_group_servers is None else frozenset(access_group_servers)
    capped = _agent_capped_servers(input_value["allowed"], input_value["agent_servers"], ceiling)
    return None if capped is None else list(capped)
