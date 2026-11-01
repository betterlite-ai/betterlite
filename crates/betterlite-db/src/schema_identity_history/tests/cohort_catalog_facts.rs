use rstest::rstest;

const DELETED_TEAM_TABLE_COLUMNS: [ColumnSpec; 27] = [
    ("id", "text", false, None),
    ("team_id", "text", false, None),
    ("team_alias", "text", true, None),
    ("organization_id", "text", true, None),
    ("object_permission_id", "text", true, None),
    ("admins", "ARRAY", true, None),
    ("members", "ARRAY", true, None),
    ("members_with_roles", "jsonb", false, Some("{}")),
    ("metadata", "jsonb", false, Some("{}")),
    ("max_budget", "double precision", true, None),
    ("spend", "double precision", false, Some("0.0")),
    ("models", "ARRAY", true, None),
    ("max_parallel_requests", "integer", true, None),
    ("tpm_limit", "bigint", true, None),
    ("rpm_limit", "bigint", true, None),
    ("budget_duration", "text", true, None),
    ("budget_reset_at", "timestamp without time zone", true, None),
    ("blocked", "boolean", false, Some("false")),
    ("model_spend", "jsonb", false, Some("{}")),
    ("model_max_budget", "jsonb", false, Some("{}")),
    ("team_member_permissions", "ARRAY", true, Some("ARRAY")),
    ("model_id", "integer", true, None),
    ("created_at", "timestamp without time zone", true, None),
    ("updated_at", "timestamp without time zone", true, None),
    (
        "deleted_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
    ),
    ("deleted_by", "text", true, None),
    ("deleted_by_api_key", "text", true, None),
];

const DELETED_TEAM_TABLE_INDEXES: [(&str, &str); 5] = [
    ("LiteLLM_DeletedTeamTable_team_id_idx", "team_id"),
    ("LiteLLM_DeletedTeamTable_deleted_at_idx", "deleted_at"),
    (
        "LiteLLM_DeletedTeamTable_organization_id_idx",
        "organization_id",
    ),
    ("LiteLLM_DeletedTeamTable_team_alias_idx", "team_alias"),
    ("LiteLLM_DeletedTeamTable_created_at_idx", "created_at"),
];

const DELETED_VERIFICATION_TOKEN_COLUMNS: [ColumnSpec; 38] = [
    ("id", "text", false, None),
    ("token", "text", false, None),
    ("key_name", "text", true, None),
    ("key_alias", "text", true, None),
    ("soft_budget_cooldown", "boolean", false, Some("false")),
    ("spend", "double precision", false, Some("0.0")),
    ("expires", "timestamp without time zone", true, None),
    ("models", "ARRAY", true, None),
    ("aliases", "jsonb", false, Some("{}")),
    ("config", "jsonb", false, Some("{}")),
    ("user_id", "text", true, None),
    ("team_id", "text", true, None),
    ("permissions", "jsonb", false, Some("{}")),
    ("max_parallel_requests", "integer", true, None),
    ("metadata", "jsonb", false, Some("{}")),
    ("blocked", "boolean", true, None),
    ("tpm_limit", "bigint", true, None),
    ("rpm_limit", "bigint", true, None),
    ("max_budget", "double precision", true, None),
    ("budget_duration", "text", true, None),
    ("budget_reset_at", "timestamp without time zone", true, None),
    ("allowed_cache_controls", "ARRAY", true, Some("ARRAY")),
    ("allowed_routes", "ARRAY", true, Some("ARRAY")),
    ("model_spend", "jsonb", false, Some("{}")),
    ("model_max_budget", "jsonb", false, Some("{}")),
    ("budget_id", "text", true, None),
    ("organization_id", "text", true, None),
    ("object_permission_id", "text", true, None),
    ("created_at", "timestamp without time zone", true, None),
    ("created_by", "text", true, None),
    ("updated_at", "timestamp without time zone", true, None),
    ("updated_by", "text", true, None),
    ("rotation_count", "integer", true, Some("0")),
    ("auto_rotate", "boolean", true, Some("false")),
    ("rotation_interval", "text", true, None),
    (
        "last_rotation_at",
        "timestamp without time zone",
        true,
        None,
    ),
    ("key_rotation_at", "timestamp without time zone", true, None),
    (
        "deleted_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
    ),
];

const DELETED_VERIFICATION_TOKEN_INDEXES: [(&str, &str); 6] = [
    ("LiteLLM_DeletedVerificationToken_token_idx", "token"),
    (
        "LiteLLM_DeletedVerificationToken_deleted_at_idx",
        "deleted_at",
    ),
    ("LiteLLM_DeletedVerificationToken_user_id_idx", "user_id"),
    ("LiteLLM_DeletedVerificationToken_team_id_idx", "team_id"),
    (
        "LiteLLM_DeletedVerificationToken_organization_id_idx",
        "organization_id",
    ),
    (
        "LiteLLM_DeletedVerificationToken_key_alias_idx",
        "key_alias",
    ),
];

const SKILLS_TABLE_COLUMNS: [ColumnSpec; 14] = [
    ("skill_id", "text", false, None),
    ("display_title", "text", true, None),
    ("description", "text", true, None),
    ("instructions", "text", true, None),
    ("source", "text", false, Some("'custom'")),
    ("latest_version", "text", true, None),
    ("file_content", "bytea", true, None),
    ("file_name", "text", true, None),
    ("file_type", "text", true, None),
    ("metadata", "jsonb", true, Some("{}")),
    (
        "created_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
    ),
    ("created_by", "text", true, None),
    (
        "updated_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
    ),
    ("updated_by", "text", true, None),
];

const MCP_SERVER_TABLE_COLUMNS: [ColumnSpec; 5] = [
    ("authorization_url", "text", true, None),
    ("registration_url", "text", true, None),
    ("token_url", "text", true, None),
    ("allow_all_keys", "boolean", false, Some("false")),
    (
        "available_on_public_internet",
        "boolean",
        false,
        Some("false"),
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
    ("policy_id", "text", false, None),
    ("policy_name", "text", false, None),
    ("inherit", "text", true, None),
    ("description", "text", true, None),
    ("guardrails_add", "ARRAY", true, Some("ARRAY")),
    ("guardrails_remove", "ARRAY", true, Some("ARRAY")),
    ("condition", "jsonb", true, Some("{}")),
    (
        "created_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
    ),
    ("created_by", "text", true, None),
    (
        "updated_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
    ),
    ("updated_by", "text", true, None),
    ("pipeline", "jsonb", true, None),
];

const POLICY_ATTACHMENT_TABLE_COLUMNS: [ColumnSpec; 11] = [
    ("attachment_id", "text", false, None),
    ("policy_name", "text", false, None),
    ("scope", "text", true, None),
    ("teams", "ARRAY", true, Some("ARRAY")),
    ("keys", "ARRAY", true, Some("ARRAY")),
    ("models", "ARRAY", true, Some("ARRAY")),
    (
        "created_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
    ),
    ("created_by", "text", true, None),
    (
        "updated_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
    ),
    ("updated_by", "text", true, None),
    ("tags", "ARRAY", true, Some("ARRAY")),
];

const MANAGED_VECTOR_STORE_TABLE_COLUMNS: [ColumnSpec; 11] = [
    ("id", "text", false, None),
    ("unified_resource_id", "text", false, None),
    ("resource_object", "jsonb", true, None),
    ("model_mappings", "jsonb", false, None),
    ("flat_model_resource_ids", "ARRAY", true, Some("ARRAY")),
    ("storage_backend", "text", true, None),
    ("storage_url", "text", true, None),
    (
        "created_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
    ),
    ("created_by", "text", true, None),
    ("updated_at", "timestamp without time zone", false, None),
    ("updated_by", "text", true, None),
];

const DEPRECATED_VERIFICATION_TOKEN_COLUMNS: [ColumnSpec; 5] = [
    ("id", "text", false, None),
    ("token", "text", false, None),
    ("active_token_id", "text", false, None),
    ("revoke_at", "timestamp without time zone", false, None),
    (
        "created_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
    ),
];

const ACCESS_GROUP_HISTORY_TABLES: [&str; 4] = [
    "LiteLLM_DeletedTeamTable",
    "LiteLLM_DeletedVerificationToken",
    "LiteLLM_TeamTable",
    "LiteLLM_VerificationToken",
];

const ACCESS_GROUP_TABLE_COLUMNS: [ColumnSpec; 11] = [
    ("access_group_id", "text", false, None),
    ("access_group_name", "text", false, None),
    ("description", "text", true, None),
    ("access_model_ids", "ARRAY", true, Some("ARRAY")),
    ("access_mcp_server_ids", "ARRAY", true, Some("ARRAY")),
    ("access_agent_ids", "ARRAY", true, Some("ARRAY")),
    ("assigned_team_ids", "ARRAY", true, Some("ARRAY")),
    ("assigned_key_ids", "ARRAY", true, Some("ARRAY")),
    (
        "created_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
    ),
    ("created_by", "text", true, None),
    (
        "updated_at",
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
    ),
];

const KEY_HISTORY_TABLES: [&str; 2] = [
    "LiteLLM_DeletedVerificationToken",
    "LiteLLM_VerificationToken",
];

async fn assert_deleted_history_defaults(client: &Client) {
    assert_columns(
        client,
        "LiteLLM_DeletedTeamTable",
        &DELETED_TEAM_TABLE_COLUMNS,
    )
    .await;
    for (name, column) in DELETED_TEAM_TABLE_INDEXES {
        assert_index(client, name, &[column], false).await;
    }
    assert_columns(
        client,
        "LiteLLM_DeletedVerificationToken",
        &DELETED_VERIFICATION_TOKEN_COLUMNS,
    )
    .await;
    for (name, column) in DELETED_VERIFICATION_TOKEN_INDEXES {
        assert_index(client, name, &[column], false).await;
    }
}

async fn assert_skills_storage(client: &Client) {
    assert_columns(client, "LiteLLM_SkillsTable", &SKILLS_TABLE_COLUMNS).await;
}

async fn assert_mcp_metadata_defaults(client: &Client) {
    assert_columns(client, "LiteLLM_MCPServerTable", &MCP_SERVER_TABLE_COLUMNS).await;
}

async fn assert_router_settings_defaults(client: &Client) {
    for table in ROUTER_SETTINGS_TABLES {
        assert_columns(
            client,
            table,
            &[("router_settings", "jsonb", true, Some("{}"))],
        )
        .await;
    }
}

async fn assert_team_guardrail_defaults(client: &Client) {
    for table in TEAM_GUARDRAIL_TABLES {
        assert_columns(
            client,
            table,
            &[(
                "allow_team_guardrail_config",
                "boolean",
                false,
                Some("false"),
            )],
        )
        .await;
    }
}

async fn assert_team_soft_budget_nullable(client: &Client) {
    for table in TEAM_SOFT_BUDGET_TABLES {
        assert_columns(
            client,
            table,
            &[("soft_budget", "double precision", true, None)],
        )
        .await;
    }
}

async fn assert_daily_endpoint_indexes(client: &Client) {
    for (table, unique_index, entity) in DAILY_ENDPOINT_COHORT {
        assert_columns(client, table, &[("endpoint", "text", true, None)]).await;
        assert_index(
            client,
            &format!("{table}_endpoint_idx"),
            &["endpoint"],
            false,
        )
        .await;
        assert_index(
            client,
            unique_index,
            &[
                entity,
                "date",
                "api_key",
                "model",
                "custom_llm_provider",
                "mcp_namespaced_tool_name",
                "endpoint",
            ],
            true,
        )
        .await;
    }
}

async fn assert_lower_email_expression(client: &Client) {
    let name = "LiteLLM_UserTable_user_email_lower_idx";
    let indexdef: String = scalar_text(
        client,
        "SELECT indexdef FROM pg_indexes WHERE indexname = $1",
        &[&name],
    )
    .await;
    assert!(indexdef.to_lowercase().contains("lower(user_email)"));
    let unique: bool = index_row(client, name).await.unwrap().get(0);
    assert!(!unique);
}

async fn assert_policy_storage(client: &Client) {
    for table in POLICY_COHORT_TABLES {
        assert_columns(client, table, &[("policies", "ARRAY", true, Some("ARRAY"))]).await;
    }
    assert_columns(client, "LiteLLM_PolicyTable", &POLICY_TABLE_COLUMNS).await;
    assert_index(
        client,
        "LiteLLM_PolicyTable_policy_name_key",
        &["policy_name"],
        true,
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_PolicyAttachmentTable",
        &POLICY_ATTACHMENT_TABLE_COLUMNS,
    )
    .await;
}

async fn assert_vector_store_ownership(client: &Client) {
    assert_columns(
        client,
        "LiteLLM_ManagedVectorStoresTable",
        &[
            ("team_id", "text", true, None),
            ("user_id", "text", true, None),
        ],
    )
    .await;
    assert_index(
        client,
        "LiteLLM_ManagedVectorStoresTable_team_id_idx",
        &["team_id"],
        false,
    )
    .await;
    assert_index(
        client,
        "LiteLLM_ManagedVectorStoresTable_user_id_idx",
        &["user_id"],
        false,
    )
    .await;
}

async fn assert_managed_vector_store_catalog(client: &Client) {
    assert_columns(
        client,
        "LiteLLM_ManagedVectorStoreTable",
        &MANAGED_VECTOR_STORE_TABLE_COLUMNS,
    )
    .await;
    assert_index(
        client,
        "LiteLLM_ManagedVectorStoreTable_unified_resource_id_key",
        &["unified_resource_id"],
        true,
    )
    .await;
    assert_index(
        client,
        "LiteLLM_ManagedVectorStoreTable_unified_resource_id_idx",
        &["unified_resource_id"],
        false,
    )
    .await;
}

async fn assert_deprecated_token_catalog(client: &Client) {
    assert_columns(
        client,
        "LiteLLM_DeprecatedVerificationToken",
        &DEPRECATED_VERIFICATION_TOKEN_COLUMNS,
    )
    .await;
    assert_index(
        client,
        "LiteLLM_DeprecatedVerificationToken_token_key",
        &["token"],
        true,
    )
    .await;
    assert_index(
        client,
        "LiteLLM_DeprecatedVerificationToken_token_revoke_at_idx",
        &["token", "revoke_at"],
        false,
    )
    .await;
    assert_index(
        client,
        "LiteLLM_DeprecatedVerificationToken_revoke_at_idx",
        &["revoke_at"],
        false,
    )
    .await;
}

async fn assert_key_lookup_indexes(client: &Client) {
    assert_index(
        client,
        "LiteLLM_VerificationToken_user_id_team_id_idx",
        &["user_id", "team_id"],
        false,
    )
    .await;
    assert_index(
        client,
        "LiteLLM_VerificationToken_team_id_idx",
        &["team_id"],
        false,
    )
    .await;
    assert_index(
        client,
        "LiteLLM_VerificationToken_budget_reset_at_expires_idx",
        &["budget_reset_at", "expires"],
        false,
    )
    .await;
}

async fn assert_access_group_history(client: &Client) {
    for table in ACCESS_GROUP_HISTORY_TABLES {
        assert_columns(
            client,
            table,
            &[("access_group_ids", "ARRAY", true, Some("ARRAY"))],
        )
        .await;
    }
    assert_columns(
        client,
        "LiteLLM_AccessGroupTable",
        &ACCESS_GROUP_TABLE_COLUMNS,
    )
    .await;
    assert_index(
        client,
        "LiteLLM_AccessGroupTable_access_group_name_key",
        &["access_group_name"],
        true,
    )
    .await;
    batch_execute(
        client,
        migration_sql_of("20260213170952_access_group_change_to_model_name"),
    )
    .await;
    assert_access_group_history_after_change(client).await;
}

async fn assert_access_group_history_after_change(client: &Client) {
    assert!(!column_exists(client, "LiteLLM_AccessGroupTable", "access_model_ids").await);
    assert_columns(
        client,
        "LiteLLM_AccessGroupTable",
        &[("access_model_names", "ARRAY", true, Some("ARRAY"))],
    )
    .await;
}

async fn assert_guardrail_owner(client: &Client) {
    assert_columns(
        client,
        "LiteLLM_GuardrailsTable",
        &[("team_id", "text", true, None)],
    )
    .await;
}

async fn assert_end_user_permission_fk(client: &Client) {
    assert_columns(
        client,
        "LiteLLM_EndUserTable",
        &[("object_permission_id", "text", true, None)],
    )
    .await;
    assert_rule(
        client,
        "LiteLLM_EndUserTable_object_permission_id_fkey",
        'c',
        'n',
    )
    .await;
}

async fn assert_key_history_additions(client: &Client) {
    for table in KEY_HISTORY_TABLES {
        assert_columns(
            client,
            table,
            &[("last_active", "timestamp without time zone", true, None)],
        )
        .await;
    }
    assert_columns(
        client,
        "LiteLLM_DeletedVerificationToken",
        &[("project_id", "text", true, None)],
    )
    .await;
}

#[derive(Clone, Copy)]
enum Cohort {
    DeletedHistoryDefaults,
    SkillsStorage,
    McpMetadataDefaults,
    RouterSettingsDefaults,
    TeamGuardrailDefaults,
    TeamSoftBudgetNullable,
    DailyEndpointIndexes,
    LowerEmailExpression,
    PolicyStorage,
    VectorStoreOwnership,
    ManagedVectorStoreCatalog,
    DeprecatedTokenCatalog,
    KeyLookupIndexes,
    AccessGroupHistory,
    GuardrailOwner,
    EndUserPermissionFk,
    KeyHistoryAdditions,
}

fn checkpoint_for(cohort: Cohort) -> &'static str {
    match cohort {
        Cohort::DeletedHistoryDefaults => {
            "20251219110931_add_deleted_keys_and_deleted_teams_tables"
        }
        Cohort::SkillsStorage => "20251220144550_schema_update",
        Cohort::McpMetadataDefaults => {
            "20260207093506_add_available_on_public_internet_to_mcp_servers"
        }
        Cohort::RouterSettingsDefaults => {
            "20260116142756_update_deleted_keys_teams_table_routing_settings"
        }
        Cohort::TeamGuardrailDefaults => "20260205091235_allow_team_guardrail_config",
        Cohort::TeamSoftBudgetNullable => "20260207110613_add_soft_budget_to_deleted_teams_table",
        Cohort::DailyEndpointIndexes => "20260106155622_add_endpoint_to_daily_activity_tables",
        Cohort::LowerEmailExpression => "20260108_add_user_email_lower_idx",
        Cohort::PolicyStorage => "20260214163027_add_pipeline_to_policy_table",
        Cohort::VectorStoreOwnership => "20260131150814_add_team_user_to_vector_stores",
        Cohort::ManagedVectorStoreCatalog => "20260213105436_add_managed_vector_store_table",
        Cohort::DeprecatedTokenCatalog => "20260203120000_add_deprecated_verification_token_table",
        Cohort::KeyLookupIndexes => "20260209085821_add_verificationtoken_indexes",
        Cohort::AccessGroupHistory => "20260212143306_add_access_group_table",
        Cohort::GuardrailOwner => "20260214094754_schema_sync",
        Cohort::EndUserPermissionFk => "20260214185341_object_permissions_for_end_users",
        Cohort::KeyHistoryAdditions => "20260219105005_add_project_id_to_deleted_keys",
    }
}

async fn run_cohort_check(client: &Client, cohort: Cohort) {
    match cohort {
        Cohort::DeletedHistoryDefaults
        | Cohort::SkillsStorage
        | Cohort::McpMetadataDefaults
        | Cohort::RouterSettingsDefaults
        | Cohort::TeamGuardrailDefaults
        | Cohort::TeamSoftBudgetNullable
        | Cohort::DailyEndpointIndexes
        | Cohort::LowerEmailExpression
        | Cohort::PolicyStorage => run_cohort_check_defaults_and_indexes(client, cohort).await,
        Cohort::VectorStoreOwnership
        | Cohort::ManagedVectorStoreCatalog
        | Cohort::DeprecatedTokenCatalog
        | Cohort::KeyLookupIndexes
        | Cohort::AccessGroupHistory
        | Cohort::GuardrailOwner
        | Cohort::EndUserPermissionFk
        | Cohort::KeyHistoryAdditions => run_cohort_check_catalog_and_history(client, cohort).await,
    }
}

async fn run_cohort_check_defaults_and_indexes(client: &Client, cohort: Cohort) {
    match cohort {
        Cohort::DeletedHistoryDefaults => assert_deleted_history_defaults(client).await,
        Cohort::SkillsStorage => assert_skills_storage(client).await,
        Cohort::McpMetadataDefaults => assert_mcp_metadata_defaults(client).await,
        Cohort::RouterSettingsDefaults => assert_router_settings_defaults(client).await,
        Cohort::TeamGuardrailDefaults => assert_team_guardrail_defaults(client).await,
        Cohort::TeamSoftBudgetNullable => assert_team_soft_budget_nullable(client).await,
        Cohort::DailyEndpointIndexes => assert_daily_endpoint_indexes(client).await,
        Cohort::LowerEmailExpression => assert_lower_email_expression(client).await,
        Cohort::PolicyStorage => assert_policy_storage(client).await,
        Cohort::VectorStoreOwnership
        | Cohort::ManagedVectorStoreCatalog
        | Cohort::DeprecatedTokenCatalog
        | Cohort::KeyLookupIndexes
        | Cohort::AccessGroupHistory
        | Cohort::GuardrailOwner
        | Cohort::EndUserPermissionFk
        | Cohort::KeyHistoryAdditions => (),
    }
}

async fn run_cohort_check_catalog_and_history(client: &Client, cohort: Cohort) {
    match cohort {
        Cohort::VectorStoreOwnership => assert_vector_store_ownership(client).await,
        Cohort::ManagedVectorStoreCatalog => assert_managed_vector_store_catalog(client).await,
        Cohort::DeprecatedTokenCatalog => assert_deprecated_token_catalog(client).await,
        Cohort::KeyLookupIndexes => assert_key_lookup_indexes(client).await,
        Cohort::AccessGroupHistory => assert_access_group_history(client).await,
        Cohort::GuardrailOwner => assert_guardrail_owner(client).await,
        Cohort::EndUserPermissionFk => assert_end_user_permission_fk(client).await,
        Cohort::KeyHistoryAdditions => assert_key_history_additions(client).await,
        Cohort::DeletedHistoryDefaults
        | Cohort::SkillsStorage
        | Cohort::McpMetadataDefaults
        | Cohort::RouterSettingsDefaults
        | Cohort::TeamGuardrailDefaults
        | Cohort::TeamSoftBudgetNullable
        | Cohort::DailyEndpointIndexes
        | Cohort::LowerEmailExpression
        | Cohort::PolicyStorage => (),
    }
}

#[rstest]
#[case::deleted_history_defaults(Cohort::DeletedHistoryDefaults)]
#[case::skills_storage(Cohort::SkillsStorage)]
#[case::mcp_metadata_defaults(Cohort::McpMetadataDefaults)]
#[case::router_settings_defaults(Cohort::RouterSettingsDefaults)]
#[case::team_guardrail_defaults(Cohort::TeamGuardrailDefaults)]
#[case::team_soft_budget_nullable(Cohort::TeamSoftBudgetNullable)]
#[case::daily_endpoint_indexes(Cohort::DailyEndpointIndexes)]
#[case::lower_email_expression(Cohort::LowerEmailExpression)]
#[case::policy_storage(Cohort::PolicyStorage)]
#[case::vector_store_ownership(Cohort::VectorStoreOwnership)]
#[case::managed_vector_store_catalog(Cohort::ManagedVectorStoreCatalog)]
#[case::deprecated_token_catalog(Cohort::DeprecatedTokenCatalog)]
#[case::key_lookup_indexes(Cohort::KeyLookupIndexes)]
#[case::access_group_history(Cohort::AccessGroupHistory)]
#[case::guardrail_owner(Cohort::GuardrailOwner)]
#[case::end_user_permission_fk(Cohort::EndUserPermissionFk)]
#[case::key_history_additions(Cohort::KeyHistoryAdditions)]
#[tokio::test]
async fn cohort_catalog_facts(#[case] cohort: Cohort) {
    harness(move |[client]| async move {
        let sql = migration_sql_through(checkpoint_for(cohort));
        batch_execute(&client, &sql).await;
        run_cohort_check(&client, cohort).await;
    })
    .await;
}
