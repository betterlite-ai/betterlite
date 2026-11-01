use rstest::rstest;

fn column_names(columns: &[ColumnSpec]) -> Vec<&'static str> {
    columns.iter().map(|spec| spec.0).collect()
}

async fn assert_deleted_history_defaults(client: &Client) {
    assert_columns(
        client,
        "LiteLLM_DeletedTeamTable",
        &DELETED_TEAM_TABLE_COLUMNS,
    )
    .await;
    assert_column_order(
        client,
        "LiteLLM_DeletedTeamTable",
        &column_names(&DELETED_TEAM_TABLE_COLUMNS),
    )
    .await;
    assert_indexes(client, &DELETED_TEAM_TABLE_INDEXES).await;
    assert_columns(
        client,
        "LiteLLM_DeletedVerificationToken",
        &DELETED_VERIFICATION_TOKEN_COLUMNS,
    )
    .await;
    assert_column_order(
        client,
        "LiteLLM_DeletedVerificationToken",
        &column_names(&DELETED_VERIFICATION_TOKEN_COLUMNS),
    )
    .await;
    assert_indexes(client, &DELETED_VERIFICATION_TOKEN_INDEXES).await;
}

async fn assert_daily_endpoint_entry(
    client: &Client,
    table: &str,
    unique_index: &str,
    entity: &str,
) {
    assert_columns(
        client,
        table,
        &[("endpoint", "text", true, None, "text", None)],
    )
    .await;
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

async fn assert_daily_endpoint_indexes(client: &Client) {
    for (table, unique_index, entity) in DAILY_ENDPOINT_COHORT {
        assert_daily_endpoint_entry(client, table, unique_index, entity).await;
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
        assert_columns(
            client,
            table,
            &[(
                "policies",
                "ARRAY",
                true,
                Some("ARRAY[]::text[]"),
                "_text",
                None,
            )],
        )
        .await;
    }
    assert_columns(client, "LiteLLM_PolicyTable", &POLICY_TABLE_COLUMNS).await;
    assert_index(client, "LiteLLM_PolicyTable_pkey", &["policy_id"], true).await;
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
    assert_index(
        client,
        "LiteLLM_PolicyAttachmentTable_pkey",
        &["attachment_id"],
        true,
    )
    .await;
}

async fn assert_access_group_catalog(client: &Client) {
    assert_single_column(
        client,
        &ACCESS_GROUP_HISTORY_TABLES,
        (
            "access_group_ids",
            "ARRAY",
            true,
            Some("ARRAY[]::text[]"),
            "_text",
            None,
        ),
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_AccessGroupTable",
        &ACCESS_GROUP_TABLE_COLUMNS,
    )
    .await;
    assert_index(
        client,
        "LiteLLM_AccessGroupTable_pkey",
        &["access_group_id"],
        true,
    )
    .await;
    assert_index(
        client,
        "LiteLLM_AccessGroupTable_access_group_name_key",
        &["access_group_name"],
        true,
    )
    .await;
}

async fn assert_access_group_model_names_migration(client: &Client) {
    batch_execute(
        client,
        migration_sql_of("20260213170952_access_group_change_to_model_name"),
    )
    .await;
    assert!(!column_exists(client, "LiteLLM_AccessGroupTable", "access_model_ids").await);
    assert_columns(
        client,
        "LiteLLM_AccessGroupTable",
        &[(
            "access_model_names",
            "ARRAY",
            true,
            Some("ARRAY[]::text[]"),
            "_text",
            None,
        )],
    )
    .await;
}

async fn assert_access_group_history(client: &Client) {
    assert_access_group_catalog(client).await;
    assert_access_group_model_names_migration(client).await;
}

async fn assert_end_user_permission_fk(client: &Client) {
    assert_columns(
        client,
        "LiteLLM_EndUserTable",
        &[("object_permission_id", "text", true, None, "text", None)],
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
    assert_single_column(
        client,
        &KEY_HISTORY_TABLES,
        (
            "last_active",
            "timestamp without time zone",
            true,
            None,
            "timestamp",
            Some(3_i32),
        ),
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_DeletedVerificationToken",
        &[("project_id", "text", true, None, "text", None)],
    )
    .await;
}

#[derive(Clone, Copy)]
enum Cohort {
    DeletedHistoryDefaults,
    TableColumns(&'static str, &'static str, &'static [ColumnSpec]),
    SingleColumn(&'static str, &'static [&'static str], ColumnSpec),
    TableCatalog(
        &'static str,
        &'static str,
        &'static [ColumnSpec],
        &'static [IndexSpec],
    ),
    DailyEndpointIndexes,
    LowerEmailExpression,
    PolicyStorage,
    KeyLookupIndexes,
    AccessGroupHistory,
    EndUserPermissionFk,
    KeyHistoryAdditions,
}

fn checkpoint_for(cohort: Cohort) -> &'static str {
    match cohort {
        Cohort::DeletedHistoryDefaults => {
            "20251219110931_add_deleted_keys_and_deleted_teams_tables"
        }
        Cohort::TableColumns(checkpoint, _, _) | Cohort::SingleColumn(checkpoint, _, _) => {
            checkpoint
        }
        Cohort::TableCatalog(checkpoint, _, _, _) => checkpoint,
        Cohort::DailyEndpointIndexes => "20260106155622_add_endpoint_to_daily_activity_tables",
        Cohort::LowerEmailExpression => "20260108_add_user_email_lower_idx",
        Cohort::PolicyStorage => "20260214163027_add_pipeline_to_policy_table",
        Cohort::KeyLookupIndexes => "20260209085821_add_verificationtoken_indexes",
        Cohort::AccessGroupHistory => "20260212143306_add_access_group_table",
        Cohort::EndUserPermissionFk => "20260214185341_object_permissions_for_end_users",
        Cohort::KeyHistoryAdditions => "20260219105005_add_project_id_to_deleted_keys",
    }
}

async fn run_cohort_check(client: &Client, cohort: Cohort) {
    match cohort {
        Cohort::DeletedHistoryDefaults => assert_deleted_history_defaults(client).await,
        Cohort::TableColumns(_, table, columns) => assert_columns(client, table, columns).await,
        Cohort::SingleColumn(_, tables, spec) => assert_single_column(client, tables, spec).await,
        Cohort::TableCatalog(_, table, columns, indexes) => {
            assert_table_catalog(client, table, columns, indexes).await;
        }
        Cohort::DailyEndpointIndexes => assert_daily_endpoint_indexes(client).await,
        Cohort::LowerEmailExpression => assert_lower_email_expression(client).await,
        Cohort::PolicyStorage => assert_policy_storage(client).await,
        Cohort::KeyLookupIndexes => assert_indexes(client, &KEY_LOOKUP_INDEXES).await,
        Cohort::AccessGroupHistory => assert_access_group_history(client).await,
        Cohort::EndUserPermissionFk => assert_end_user_permission_fk(client).await,
        Cohort::KeyHistoryAdditions => assert_key_history_additions(client).await,
    }
}

const ROUTER_SETTINGS_CHECKPOINT: &str =
    "20260116142756_update_deleted_keys_teams_table_routing_settings";
const ROUTER_SETTINGS_SPEC: ColumnSpec = (
    "router_settings",
    "jsonb",
    true,
    Some("'{}'::jsonb"),
    "jsonb",
    None,
);

const TEAM_GUARDRAIL_CHECKPOINT: &str = "20260205091235_allow_team_guardrail_config";
const TEAM_GUARDRAIL_SPEC: ColumnSpec = (
    "allow_team_guardrail_config",
    "boolean",
    false,
    Some("false"),
    "bool",
    None,
);

const TEAM_SOFT_BUDGET_CHECKPOINT: &str = "20260207110613_add_soft_budget_to_deleted_teams_table";
const TEAM_SOFT_BUDGET_SPEC: ColumnSpec = (
    "soft_budget",
    "double precision",
    true,
    None,
    "float8",
    None,
);

const GUARDRAIL_OWNER_CHECKPOINT: &str = "20260214094754_schema_sync";
const GUARDRAIL_OWNER_SPEC: ColumnSpec = ("team_id", "text", true, None, "text", None);

const SKILLS_STORAGE_CHECKPOINT: &str = "20251220144550_schema_update";
const MCP_METADATA_CHECKPOINT: &str =
    "20260207093506_add_available_on_public_internet_to_mcp_servers";

const VECTOR_STORE_OWNERSHIP_CHECKPOINT: &str = "20260131150814_add_team_user_to_vector_stores";
const MANAGED_VECTOR_STORE_CHECKPOINT: &str = "20260213105436_add_managed_vector_store_table";
const DEPRECATED_TOKEN_CHECKPOINT: &str = "20260203120000_add_deprecated_verification_token_table";

#[rstest]
#[case::deleted_history_defaults(Cohort::DeletedHistoryDefaults)]
#[case::skills_storage(Cohort::TableCatalog(
    SKILLS_STORAGE_CHECKPOINT,
    "LiteLLM_SkillsTable",
    &SKILLS_TABLE_COLUMNS,
    &SKILLS_TABLE_INDEXES
))]
#[case::mcp_metadata_defaults(Cohort::TableColumns(
    MCP_METADATA_CHECKPOINT,
    "LiteLLM_MCPServerTable",
    &MCP_SERVER_TABLE_COLUMNS
))]
#[case::router_settings_defaults(Cohort::SingleColumn(
    ROUTER_SETTINGS_CHECKPOINT,
    &ROUTER_SETTINGS_TABLES,
    ROUTER_SETTINGS_SPEC
))]
#[case::team_guardrail_defaults(Cohort::SingleColumn(
    TEAM_GUARDRAIL_CHECKPOINT,
    &TEAM_GUARDRAIL_TABLES,
    TEAM_GUARDRAIL_SPEC
))]
#[case::team_soft_budget_nullable(Cohort::SingleColumn(
    TEAM_SOFT_BUDGET_CHECKPOINT,
    &TEAM_SOFT_BUDGET_TABLES,
    TEAM_SOFT_BUDGET_SPEC
))]
#[case::daily_endpoint_indexes(Cohort::DailyEndpointIndexes)]
#[case::lower_email_expression(Cohort::LowerEmailExpression)]
#[case::policy_storage(Cohort::PolicyStorage)]
#[case::vector_store_ownership(Cohort::TableCatalog(
    VECTOR_STORE_OWNERSHIP_CHECKPOINT,
    "LiteLLM_ManagedVectorStoresTable",
    &VECTOR_STORE_OWNERSHIP_COLUMNS,
    &VECTOR_STORE_OWNERSHIP_INDEXES
))]
#[case::managed_vector_store_catalog(Cohort::TableCatalog(
    MANAGED_VECTOR_STORE_CHECKPOINT,
    "LiteLLM_ManagedVectorStoreTable",
    &MANAGED_VECTOR_STORE_TABLE_COLUMNS,
    &MANAGED_VECTOR_STORE_INDEXES
))]
#[case::deprecated_token_catalog(Cohort::TableCatalog(
    DEPRECATED_TOKEN_CHECKPOINT,
    "LiteLLM_DeprecatedVerificationToken",
    &DEPRECATED_VERIFICATION_TOKEN_COLUMNS,
    &DEPRECATED_TOKEN_INDEXES
))]
#[case::key_lookup_indexes(Cohort::KeyLookupIndexes)]
#[case::access_group_history(Cohort::AccessGroupHistory)]
#[case::guardrail_owner(Cohort::SingleColumn(
    GUARDRAIL_OWNER_CHECKPOINT,
    &GUARDRAIL_OWNER_TABLES,
    GUARDRAIL_OWNER_SPEC
))]
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
