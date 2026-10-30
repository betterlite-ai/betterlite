CREATE TEMP TABLE "LiteLLM_ObjectPermissionTable" (
    "object_permission_id" TEXT NOT NULL PRIMARY KEY,
    "mcp_servers" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "mcp_access_groups" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "mcp_tool_permissions" JSONB,
    "vector_stores" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "agents" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "agent_access_groups" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "models" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "mcp_toolsets" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "blocked_tools" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "search_tools" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "mcp_tool_search_enabled" BOOLEAN,
    "skills" TEXT[] DEFAULT ARRAY[]::TEXT[]
);

INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id, models)
VALUES ('null', NULL), ('empty', ARRAY[]::TEXT[]);
INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id) VALUES ('omitted');
