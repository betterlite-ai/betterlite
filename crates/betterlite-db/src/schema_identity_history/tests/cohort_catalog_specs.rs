const DELETED_TEAM_TABLE_COLUMNS: [ColumnSpec; 28] = [
    ("id", "text", false, None, "text", None),
    ("team_id", "text", false, None, "text", None),
    ("team_alias", "text", true, None, "text", None),
    ("organization_id", "text", true, None, "text", None),
    ("object_permission_id", "text", true, None, "text", None),
    ("admins", "ARRAY", true, None, "_text", None),
    ("members", "ARRAY", true, None, "_text", None),
    (
        "members_with_roles",
        "jsonb",
        false,
        Some("'{}'::jsonb"),
        "jsonb",
        None,
    ),
    (
        "metadata",
        "jsonb",
        false,
        Some("'{}'::jsonb"),
        "jsonb",
        None,
    ),
    ("max_budget", "double precision", true, None, "float8", None),
    (
        "spend",
        "double precision",
        false,
        Some("0.0"),
        "float8",
        None,
    ),
    ("models", "ARRAY", true, None, "_text", None),
    ("max_parallel_requests", "integer", true, None, "int4", None),
    ("tpm_limit", "bigint", true, None, "int8", None),
    ("rpm_limit", "bigint", true, None, "int8", None),
    ("budget_duration", "text", true, None, "text", None),
    (
        "budget_reset_at",
        "timestamp without time zone",
        true,
        None,
        "timestamp",
        Some(3_i32),
    ),
    ("blocked", "boolean", false, Some("false"), "bool", None),
    (
        "model_spend",
        "jsonb",
        false,
        Some("'{}'::jsonb"),
        "jsonb",
        None,
    ),
    (
        "model_max_budget",
        "jsonb",
        false,
        Some("'{}'::jsonb"),
        "jsonb",
        None,
    ),
    (
        "team_member_permissions",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
    ("model_id", "integer", true, None, "int4", None),
    (
        "created_at",
        "timestamp without time zone",
        true,
        None,
        "timestamp",
        Some(3_i32),
    ),
    (
        "updated_at",
        "timestamp without time zone",
        true,
        None,
        "timestamp",
        Some(3_i32),
    ),
    (
        "deleted_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
        "timestamp",
        Some(3_i32),
    ),
    ("deleted_by", "text", true, None, "text", None),
    ("deleted_by_api_key", "text", true, None, "text", None),
    ("litellm_changed_by", "text", true, None, "text", None),
];

const DELETED_TEAM_TABLE_INDEXES: [IndexSpec; 6] = [
    ("LiteLLM_DeletedTeamTable_pkey", &["id"], true),
    ("LiteLLM_DeletedTeamTable_team_id_idx", &["team_id"], false),
    (
        "LiteLLM_DeletedTeamTable_deleted_at_idx",
        &["deleted_at"],
        false,
    ),
    (
        "LiteLLM_DeletedTeamTable_organization_id_idx",
        &["organization_id"],
        false,
    ),
    (
        "LiteLLM_DeletedTeamTable_team_alias_idx",
        &["team_alias"],
        false,
    ),
    (
        "LiteLLM_DeletedTeamTable_created_at_idx",
        &["created_at"],
        false,
    ),
];

const DELETED_VERIFICATION_TOKEN_COLUMNS: [ColumnSpec; 41] = [
    ("id", "text", false, None, "text", None),
    ("token", "text", false, None, "text", None),
    ("key_name", "text", true, None, "text", None),
    ("key_alias", "text", true, None, "text", None),
    (
        "soft_budget_cooldown",
        "boolean",
        false,
        Some("false"),
        "bool",
        None,
    ),
    (
        "spend",
        "double precision",
        false,
        Some("0.0"),
        "float8",
        None,
    ),
    (
        "expires",
        "timestamp without time zone",
        true,
        None,
        "timestamp",
        Some(3_i32),
    ),
    ("models", "ARRAY", true, None, "_text", None),
    (
        "aliases",
        "jsonb",
        false,
        Some("'{}'::jsonb"),
        "jsonb",
        None,
    ),
    ("config", "jsonb", false, Some("'{}'::jsonb"), "jsonb", None),
    ("user_id", "text", true, None, "text", None),
    ("team_id", "text", true, None, "text", None),
    (
        "permissions",
        "jsonb",
        false,
        Some("'{}'::jsonb"),
        "jsonb",
        None,
    ),
    ("max_parallel_requests", "integer", true, None, "int4", None),
    (
        "metadata",
        "jsonb",
        false,
        Some("'{}'::jsonb"),
        "jsonb",
        None,
    ),
    ("blocked", "boolean", true, None, "bool", None),
    ("tpm_limit", "bigint", true, None, "int8", None),
    ("rpm_limit", "bigint", true, None, "int8", None),
    ("max_budget", "double precision", true, None, "float8", None),
    ("budget_duration", "text", true, None, "text", None),
    (
        "budget_reset_at",
        "timestamp without time zone",
        true,
        None,
        "timestamp",
        Some(3_i32),
    ),
    (
        "allowed_cache_controls",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
    (
        "allowed_routes",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
    (
        "model_spend",
        "jsonb",
        false,
        Some("'{}'::jsonb"),
        "jsonb",
        None,
    ),
    (
        "model_max_budget",
        "jsonb",
        false,
        Some("'{}'::jsonb"),
        "jsonb",
        None,
    ),
    ("budget_id", "text", true, None, "text", None),
    ("organization_id", "text", true, None, "text", None),
    ("object_permission_id", "text", true, None, "text", None),
    (
        "created_at",
        "timestamp without time zone",
        true,
        None,
        "timestamp",
        Some(3_i32),
    ),
    ("created_by", "text", true, None, "text", None),
    (
        "updated_at",
        "timestamp without time zone",
        true,
        None,
        "timestamp",
        Some(3_i32),
    ),
    ("updated_by", "text", true, None, "text", None),
    ("rotation_count", "integer", true, Some("0"), "int4", None),
    ("auto_rotate", "boolean", true, Some("false"), "bool", None),
    ("rotation_interval", "text", true, None, "text", None),
    (
        "last_rotation_at",
        "timestamp without time zone",
        true,
        None,
        "timestamp",
        Some(3_i32),
    ),
    (
        "key_rotation_at",
        "timestamp without time zone",
        true,
        None,
        "timestamp",
        Some(3_i32),
    ),
    (
        "deleted_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
        "timestamp",
        Some(3_i32),
    ),
    ("deleted_by", "text", true, None, "text", None),
    ("deleted_by_api_key", "text", true, None, "text", None),
    ("litellm_changed_by", "text", true, None, "text", None),
];

const DELETED_VERIFICATION_TOKEN_INDEXES: [IndexSpec; 8] = [
    ("LiteLLM_DeletedVerificationToken_pkey", &["id"], true),
    (
        "LiteLLM_DeletedVerificationToken_token_idx",
        &["token"],
        false,
    ),
    (
        "LiteLLM_DeletedVerificationToken_deleted_at_idx",
        &["deleted_at"],
        false,
    ),
    (
        "LiteLLM_DeletedVerificationToken_user_id_idx",
        &["user_id"],
        false,
    ),
    (
        "LiteLLM_DeletedVerificationToken_team_id_idx",
        &["team_id"],
        false,
    ),
    (
        "LiteLLM_DeletedVerificationToken_organization_id_idx",
        &["organization_id"],
        false,
    ),
    (
        "LiteLLM_DeletedVerificationToken_key_alias_idx",
        &["key_alias"],
        false,
    ),
    (
        "LiteLLM_DeletedVerificationToken_created_at_idx",
        &["created_at"],
        false,
    ),
];

const SKILLS_TABLE_COLUMNS: [ColumnSpec; 14] = [
    ("skill_id", "text", false, None, "text", None),
    ("display_title", "text", true, None, "text", None),
    ("description", "text", true, None, "text", None),
    ("instructions", "text", true, None, "text", None),
    (
        "source",
        "text",
        false,
        Some("'custom'::text"),
        "text",
        None,
    ),
    ("latest_version", "text", true, None, "text", None),
    ("file_content", "bytea", true, None, "bytea", None),
    ("file_name", "text", true, None, "text", None),
    ("file_type", "text", true, None, "text", None),
    (
        "metadata",
        "jsonb",
        true,
        Some("'{}'::jsonb"),
        "jsonb",
        None,
    ),
    (
        "created_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
        "timestamp",
        Some(3_i32),
    ),
    ("created_by", "text", true, None, "text", None),
    (
        "updated_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
        "timestamp",
        Some(3_i32),
    ),
    ("updated_by", "text", true, None, "text", None),
];

const SKILLS_TABLE_INDEXES: [IndexSpec; 1] = [("LiteLLM_SkillsTable_pkey", &["skill_id"], true)];

const MCP_SERVER_TABLE_COLUMNS: [ColumnSpec; 5] = [
    ("authorization_url", "text", true, None, "text", None),
    ("registration_url", "text", true, None, "text", None),
    ("token_url", "text", true, None, "text", None),
    (
        "allow_all_keys",
        "boolean",
        false,
        Some("false"),
        "bool",
        None,
    ),
    (
        "available_on_public_internet",
        "boolean",
        false,
        Some("false"),
        "bool",
        None,
    ),
];

const ROUTER_SETTINGS_TABLES: [&str; 4] = [
    "LiteLLM_TeamTable",
    "LiteLLM_VerificationToken",
    "LiteLLM_DeletedTeamTable",
    "LiteLLM_DeletedVerificationToken",
];

const TEAM_GUARDRAIL_TABLES: [&str; 2] = ["LiteLLM_DeletedTeamTable", "LiteLLM_TeamTable"];

const TEAM_SOFT_BUDGET_TABLES: [&str; 2] = ["LiteLLM_TeamTable", "LiteLLM_DeletedTeamTable"];

const GUARDRAIL_OWNER_TABLES: [&str; 1] = ["LiteLLM_GuardrailsTable"];

const DAILY_ENDPOINT_COHORT: [(&str, &str, &str); 6] = [
    (
        "LiteLLM_DailyAgentSpend",
        "LiteLLM_DailyAgentSpend_agent_id_date_api_key_model_custom__key",
        "agent_id",
    ),
    (
        "LiteLLM_DailyEndUserSpend",
        "LiteLLM_DailyEndUserSpend_end_user_id_date_api_key_model_cu_key",
        "end_user_id",
    ),
    (
        "LiteLLM_DailyOrganizationSpend",
        "LiteLLM_DailyOrganizationSpend_organization_id_date_api_key_key",
        "organization_id",
    ),
    (
        "LiteLLM_DailyTagSpend",
        "LiteLLM_DailyTagSpend_tag_date_api_key_model_custom_llm_pro_key",
        "tag",
    ),
    (
        "LiteLLM_DailyTeamSpend",
        "LiteLLM_DailyTeamSpend_team_id_date_api_key_model_custom_ll_key",
        "team_id",
    ),
    (
        "LiteLLM_DailyUserSpend",
        "LiteLLM_DailyUserSpend_user_id_date_api_key_model_custom_ll_key",
        "user_id",
    ),
];

const POLICY_COHORT_TABLES: [&str; 5] = [
    "LiteLLM_DeletedTeamTable",
    "LiteLLM_DeletedVerificationToken",
    "LiteLLM_TeamTable",
    "LiteLLM_UserTable",
    "LiteLLM_VerificationToken",
];

const POLICY_TABLE_COLUMNS: [ColumnSpec; 12] = [
    ("policy_id", "text", false, None, "text", None),
    ("policy_name", "text", false, None, "text", None),
    ("inherit", "text", true, None, "text", None),
    ("description", "text", true, None, "text", None),
    (
        "guardrails_add",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
    (
        "guardrails_remove",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
    (
        "condition",
        "jsonb",
        true,
        Some("'{}'::jsonb"),
        "jsonb",
        None,
    ),
    (
        "created_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
        "timestamp",
        Some(3_i32),
    ),
    ("created_by", "text", true, None, "text", None),
    (
        "updated_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
        "timestamp",
        Some(3_i32),
    ),
    ("updated_by", "text", true, None, "text", None),
    ("pipeline", "jsonb", true, None, "jsonb", None),
];

const POLICY_ATTACHMENT_TABLE_COLUMNS: [ColumnSpec; 11] = [
    ("attachment_id", "text", false, None, "text", None),
    ("policy_name", "text", false, None, "text", None),
    ("scope", "text", true, None, "text", None),
    (
        "teams",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
    (
        "keys",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
    (
        "models",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
    (
        "created_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
        "timestamp",
        Some(3_i32),
    ),
    ("created_by", "text", true, None, "text", None),
    (
        "updated_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
        "timestamp",
        Some(3_i32),
    ),
    ("updated_by", "text", true, None, "text", None),
    (
        "tags",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
];

const MANAGED_VECTOR_STORE_TABLE_COLUMNS: [ColumnSpec; 11] = [
    ("id", "text", false, None, "text", None),
    ("unified_resource_id", "text", false, None, "text", None),
    ("resource_object", "jsonb", true, None, "jsonb", None),
    ("model_mappings", "jsonb", false, None, "jsonb", None),
    (
        "flat_model_resource_ids",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
    ("storage_backend", "text", true, None, "text", None),
    ("storage_url", "text", true, None, "text", None),
    (
        "created_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
        "timestamp",
        Some(3_i32),
    ),
    ("created_by", "text", true, None, "text", None),
    (
        "updated_at",
        "timestamp without time zone",
        false,
        None,
        "timestamp",
        Some(3_i32),
    ),
    ("updated_by", "text", true, None, "text", None),
];

const DEPRECATED_VERIFICATION_TOKEN_COLUMNS: [ColumnSpec; 5] = [
    ("id", "text", false, None, "text", None),
    ("token", "text", false, None, "text", None),
    ("active_token_id", "text", false, None, "text", None),
    (
        "revoke_at",
        "timestamp without time zone",
        false,
        None,
        "timestamp",
        Some(3_i32),
    ),
    (
        "created_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
        "timestamp",
        Some(3_i32),
    ),
];

const ACCESS_GROUP_HISTORY_TABLES: [&str; 4] = [
    "LiteLLM_DeletedTeamTable",
    "LiteLLM_DeletedVerificationToken",
    "LiteLLM_TeamTable",
    "LiteLLM_VerificationToken",
];

const ACCESS_GROUP_TABLE_COLUMNS: [ColumnSpec; 11] = [
    ("access_group_id", "text", false, None, "text", None),
    ("access_group_name", "text", false, None, "text", None),
    ("description", "text", true, None, "text", None),
    (
        "access_model_ids",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
    (
        "access_mcp_server_ids",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
    (
        "access_agent_ids",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
    (
        "assigned_team_ids",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
    (
        "assigned_key_ids",
        "ARRAY",
        true,
        Some("ARRAY[]::text[]"),
        "_text",
        None,
    ),
    (
        "created_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
        "timestamp",
        Some(3_i32),
    ),
    ("created_by", "text", true, None, "text", None),
    (
        "updated_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
        "timestamp",
        Some(3_i32),
    ),
];

const KEY_HISTORY_TABLES: [&str; 2] = [
    "LiteLLM_DeletedVerificationToken",
    "LiteLLM_VerificationToken",
];

const VECTOR_STORE_OWNERSHIP_COLUMNS: [ColumnSpec; 2] = [
    ("team_id", "text", true, None, "text", None),
    ("user_id", "text", true, None, "text", None),
];

const VECTOR_STORE_OWNERSHIP_INDEXES: [IndexSpec; 2] = [
    (
        "LiteLLM_ManagedVectorStoresTable_team_id_idx",
        &["team_id"],
        false,
    ),
    (
        "LiteLLM_ManagedVectorStoresTable_user_id_idx",
        &["user_id"],
        false,
    ),
];

const MANAGED_VECTOR_STORE_INDEXES: [IndexSpec; 3] = [
    ("LiteLLM_ManagedVectorStoreTable_pkey", &["id"], true),
    (
        "LiteLLM_ManagedVectorStoreTable_unified_resource_id_key",
        &["unified_resource_id"],
        true,
    ),
    (
        "LiteLLM_ManagedVectorStoreTable_unified_resource_id_idx",
        &["unified_resource_id"],
        false,
    ),
];

const DEPRECATED_TOKEN_INDEXES: [IndexSpec; 4] = [
    ("LiteLLM_DeprecatedVerificationToken_pkey", &["id"], true),
    (
        "LiteLLM_DeprecatedVerificationToken_token_key",
        &["token"],
        true,
    ),
    (
        "LiteLLM_DeprecatedVerificationToken_token_revoke_at_idx",
        &["token", "revoke_at"],
        false,
    ),
    (
        "LiteLLM_DeprecatedVerificationToken_revoke_at_idx",
        &["revoke_at"],
        false,
    ),
];

const KEY_LOOKUP_INDEXES: [IndexSpec; 3] = [
    (
        "LiteLLM_VerificationToken_user_id_team_id_idx",
        &["user_id", "team_id"],
        false,
    ),
    ("LiteLLM_VerificationToken_team_id_idx", &["team_id"], false),
    (
        "LiteLLM_VerificationToken_budget_reset_at_expires_idx",
        &["budget_reset_at", "expires"],
        false,
    ),
];
