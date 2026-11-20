pub mod codec;
pub mod listing;

#[path = "agent_rows/tests/support.rs"]
mod support;

// Prisma's find_many with both includes, each joined table's columns prefixed by its alias
pub const AGENT_SELECT_SQL: &str = r#"SELECT a.*,
    i.agent_id AS i_agent_id,
    i.active AS i_active,
    i.provider AS i_provider,
    i.issuer AS i_issuer,
    i.tenant_id AS i_tenant_id,
    i.client_id AS i_client_id,
    i.service_principal_id AS i_service_principal_id,
    i.required_roles AS i_required_roles,
    i.required_scopes AS i_required_scopes,
    i.revision AS i_revision,
    i.last_authenticated_at AS i_last_authenticated_at,
    p.object_permission_id AS p_object_permission_id,
    p.mcp_servers AS p_mcp_servers,
    p.mcp_access_groups AS p_mcp_access_groups,
    p.mcp_tool_permissions AS p_mcp_tool_permissions,
    p.vector_stores AS p_vector_stores,
    p.agents AS p_agents,
    p.agent_access_groups AS p_agent_access_groups,
    p.models AS p_models,
    p.blocked_tools AS p_blocked_tools,
    p.mcp_toolsets AS p_mcp_toolsets,
    p.search_tools AS p_search_tools,
    p.mcp_tool_search_enabled AS p_mcp_tool_search_enabled,
    p.skills AS p_skills
FROM "LiteLLM_AgentsTable" a
LEFT JOIN "LiteLLM_AgentIdentity" i ON i.agent_id = a.agent_id
LEFT JOIN "LiteLLM_ObjectPermissionTable" p ON p.object_permission_id = a.object_permission_id"#;
