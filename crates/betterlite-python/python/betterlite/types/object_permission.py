from typing_extensions import ReadOnly, TypedDict


class ObjectPermissionDict(TypedDict, total=False):
    mcp_servers: list[str] | None
    mcp_access_groups: list[str] | None
    mcp_tool_permissions: dict[str, list[str]] | None
    mcp_toolsets: list[str] | None
    blocked_tools: list[str] | None
    vector_stores: list[str] | None
    agents: list[str] | None
    agent_access_groups: list[str] | None
    models: list[str] | None
    search_tools: list[str] | None
    mcp_tool_search_enabled: bool | None
    skills: ReadOnly[list[str] | None]
