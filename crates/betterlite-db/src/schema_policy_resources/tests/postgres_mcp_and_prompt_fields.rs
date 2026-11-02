const SCHEMA_SYNC_0306: &str = "20260306233848_schema_sync";
const ADD_MCP_APPROVAL_STATUS: &str = "20260309000000_add_mcp_approval_status";
const ADD_INDEX_TO_TEAM_TABLE: &str = "20260318140652_add_index_to_team_table";
const RESTORE_MCP_APPROVAL_FIELDS: &str = "20260319000000_restore_mcp_approval_fields";
const ADD_MCP_TOOLSETS: &str = "20260321000000_add_mcp_toolsets";
const ADD_PROMPT_ENVIRONMENT_AND_CREATED_BY: &str =
    "20260331000000_add_prompt_environment_and_created_by";
const ADD_BUDGET_LIMITS: &str = "20260401000000_add_budget_limits";
const ADD_TEAM_MEMBER_MODEL_SCOPE: &str = "20260401000000_add_team_member_model_scope";
const ADD_MCP_SERVER_INSTRUCTIONS: &str = "20260414140000_add_mcp_server_instructions";

async fn assert_byok_metadata_columns(client: &Client) {
    assert_columns(
        client,
        "LiteLLM_MCPServerTable",
        &[
            ("byok_api_key_help_url", "text", true, None, "text", None),
            (
                "byok_description",
                "ARRAY",
                true,
                Some("ARRAY[]::text[]"),
                "_text",
                None,
            ),
            ("is_byok", "boolean", false, Some("false"), "bool", None),
            (
                "tool_name_to_description",
                "jsonb",
                true,
                Some("'{}'::jsonb"),
                "jsonb",
                None,
            ),
            (
                "tool_name_to_display_name",
                "jsonb",
                true,
                Some("'{}'::jsonb"),
                "jsonb",
                None,
            ),
        ],
    )
    .await;
}

async fn check_byok_false_and_empty_metadata(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, url) VALUES ($1, $2)"#,
        &[&"server-byok", &"https://byok.test"],
    )
    .await;
    assert_byok_metadata_columns(client).await;
    let is_byok = scalar_bool(
        client,
        r#"SELECT is_byok FROM "LiteLLM_MCPServerTable" WHERE server_id = $1"#,
        &[&"server-byok"],
    )
    .await;
    assert!(!is_byok);
}

async fn check_mcp_user_credential_raw_text_unique_pair(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_MCPUserCredentials" (id, user_id, server_id, credential_b64) VALUES ($1, $2, $3, $4)"#,
        &[&"cred-a", &"user-a", &"server-a", &"bm90LWFjdHVhbGx5LWVuY29kZWQ="],
    )
    .await;
    let stored = scalar_text(
        client,
        r#"SELECT credential_b64 FROM "LiteLLM_MCPUserCredentials" WHERE id = $1"#,
        &[&"cred-a"],
    )
    .await;
    assert_eq!(
        stored, "bm90LWFjdHVhbGx5LWVuY29kZWQ=",
        "raw text is stored verbatim"
    );

    let conflict = exec_params_err(
        client,
        r#"INSERT INTO "LiteLLM_MCPUserCredentials" (id, user_id, server_id, credential_b64) VALUES ($1, $2, $3, $4)"#,
        &[&"cred-b", &"user-a", &"server-a", &"different-value"],
    )
    .await;
    assert_eq!(sql_state(&conflict), SqlState::UNIQUE_VIOLATION.code());
}

async fn check_jwt_mapping_active_default_and_unique_claim_pair(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token) VALUES ($1)"#,
        &[&"token-jwt-a"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_JWTKeyMapping" (id, jwt_claim_name, jwt_claim_value, token)
           VALUES ($1, $2, $3, $4)"#,
        &[&"jwt-a", &"org", &"acme", &"token-jwt-a"],
    )
    .await;
    let active = scalar_bool(
        client,
        r#"SELECT is_active FROM "LiteLLM_JWTKeyMapping" WHERE id = $1"#,
        &[&"jwt-a"],
    )
    .await;
    assert!(active);

    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token) VALUES ($1)"#,
        &[&"token-jwt-b"],
    )
    .await;
    let conflict = exec_params_err(
        client,
        r#"INSERT INTO "LiteLLM_JWTKeyMapping" (id, jwt_claim_name, jwt_claim_value, token, is_active)
           VALUES ($1, $2, $3, $4, false)"#,
        &[&"jwt-b", &"org", &"acme", &"token-jwt-b"],
    )
    .await;
    assert_eq!(
        sql_state(&conflict),
        SqlState::UNIQUE_VIOLATION.code(),
        "the claim-pair uniqueness ignores is_active"
    );
}

async fn check_jwt_token_delete_restrict_update_cascade(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token) VALUES ($1)"#,
        &[&"token-cascade"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_JWTKeyMapping" (id, jwt_claim_name, jwt_claim_value, token)
           VALUES ($1, $2, $3, $4)"#,
        &[&"jwt-cascade", &"org", &"acme", &"token-cascade"],
    )
    .await;

    assert_rule(client, "LiteLLM_JWTKeyMapping_token_fkey", 'c', 'r').await;

    let delete_error = exec_params_err(
        client,
        r#"DELETE FROM "LiteLLM_VerificationToken" WHERE token = $1"#,
        &[&"token-cascade"],
    )
    .await;
    assert_eq!(
        sql_state(&delete_error),
        SqlState::FOREIGN_KEY_VIOLATION.code()
    );

    exec_params(
        client,
        r#"UPDATE "LiteLLM_VerificationToken" SET token = $1 WHERE token = $2"#,
        &[&"token-cascade-renamed", &"token-cascade"],
    )
    .await;
    let cascaded = scalar_text(
        client,
        r#"SELECT token FROM "LiteLLM_JWTKeyMapping" WHERE id = $1"#,
        &[&"jwt-cascade"],
    )
    .await;
    assert_eq!(cascaded, "token-cascade-renamed");
}

async fn assert_config_override_insert_not_null_violation(client: &Client, sql: &str, id: &str) {
    let error = exec_params_err(client, sql, &[&id]).await;
    assert_eq!(sql_state(&error), SqlState::NOT_NULL_VIOLATION.code());
}

async fn check_config_override_required_json_and_updated_at(client: &Client) {
    assert_columns(
        client,
        "LiteLLM_ConfigOverrides",
        &[
            ("config_value", "jsonb", false, None, "jsonb", None),
            (
                "updated_at",
                "timestamp without time zone",
                false,
                None,
                "timestamp",
                Some(3_i32),
            ),
        ],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ConfigOverrides" (config_type, config_value, updated_at) VALUES ($1, '{}', now())"#,
        &[&"router-config"],
    )
    .await;
    assert_config_override_insert_not_null_violation(
        client,
        r#"INSERT INTO "LiteLLM_ConfigOverrides" (config_type, updated_at) VALUES ($1, now())"#,
        "missing-value",
    )
    .await;
    assert_config_override_insert_not_null_violation(
        client,
        r#"INSERT INTO "LiteLLM_ConfigOverrides" (config_type, config_value) VALUES ($1, '{}')"#,
        "missing-updated-at",
    )
    .await;
}

async fn check_approval_fields_restore_preserves_values(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, url, approval_status, submitted_by)
           VALUES ($1, $2, $3, $4)"#,
        &[
            &"server-approval",
            &"https://approval.test",
            &"pending",
            &"reviewer-a",
        ],
    )
    .await;
    batch_execute(client, migration_sql_of(RESTORE_MCP_APPROVAL_FIELDS)).await;
    let row = query_one_row(
        client,
        r#"SELECT approval_status, submitted_by FROM "LiteLLM_MCPServerTable" WHERE server_id = $1"#,
        &[&"server-approval"],
    )
    .await;
    let approval_status: Option<String> = row.get(0);
    let submitted_by: Option<String> = row.get(1);
    assert_eq!(approval_status, Some("pending".to_owned()));
    assert_eq!(submitted_by, Some("reviewer-a".to_owned()));
}

async fn check_team_lookup_indexes(client: &Client) {
    assert_index(
        client,
        "LiteLLM_TeamTable_organization_id_idx",
        &["organization_id"],
        false,
    )
    .await;
    assert_index(
        client,
        "LiteLLM_TeamTable_team_alias_idx",
        &["team_alias"],
        false,
    )
    .await;
    assert_index(
        client,
        "LiteLLM_TeamTable_created_at_idx",
        &["created_at"],
        false,
    )
    .await;
}

async fn check_toolset_empty_tools_unique_name(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_MCPToolsetTable" (toolset_id, toolset_name) VALUES ($1, $2)"#,
        &[&"toolset-a", &"toolset-a-name"],
    )
    .await;
    let tools = scalar_text(
        client,
        r#"SELECT tools::text FROM "LiteLLM_MCPToolsetTable" WHERE toolset_id = $1"#,
        &[&"toolset-a"],
    )
    .await;
    assert_eq!(tools, "[]");

    let conflict = exec_params_err(
        client,
        r#"INSERT INTO "LiteLLM_MCPToolsetTable" (toolset_id, toolset_name) VALUES ($1, $2)"#,
        &[&"toolset-b", &"toolset-a-name"],
    )
    .await;
    assert_eq!(sql_state(&conflict), SqlState::UNIQUE_VIOLATION.code());

    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id) VALUES ($1)"#,
        &[&"permission-toolsets"],
    )
    .await;
    let mcp_toolsets: Vec<String> = query_one_row(
        client,
        r#"SELECT mcp_toolsets FROM "LiteLLM_ObjectPermissionTable" WHERE object_permission_id = $1"#,
        &[&"permission-toolsets"],
    )
    .await
    .get(0);
    assert!(mcp_toolsets.is_empty());
}

async fn check_prompt_development_backfill(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_PromptTable" (id, prompt_id, litellm_params, updated_at)
           VALUES ($1, $2, '{}', now())"#,
        &[&"prompt-row-existing", &"prompt-existing"],
    )
    .await;
    batch_execute(
        client,
        migration_sql_of(ADD_PROMPT_ENVIRONMENT_AND_CREATED_BY),
    )
    .await;
    let environment = scalar_text(
        client,
        r#"SELECT environment FROM "LiteLLM_PromptTable" WHERE id = $1"#,
        &[&"prompt-row-existing"],
    )
    .await;
    assert_eq!(environment, "development");
}

async fn assert_prompt_environment_indexes(client: &Client) {
    assert_index(
        client,
        "LiteLLM_PromptTable_prompt_id_version_environment_key",
        &["prompt_id", "version", "environment"],
        true,
    )
    .await;
    assert_index(
        client,
        "LiteLLM_PromptTable_prompt_id_environment_idx",
        &["prompt_id", "environment"],
        false,
    )
    .await;
}

async fn check_prompt_environment_distinct_versions(client: &Client) {
    assert_prompt_environment_indexes(client).await;

    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_PromptTable" (id, prompt_id, litellm_params, updated_at, environment)
           VALUES ($1, $2, '{}', now(), 'development'), ($3, $2, '{}', now(), 'production')"#,
        &[&"prompt-dev", &"prompt-shared", &"prompt-prod"],
    )
    .await;
    let row = query_one_row(
        client,
        r#"SELECT array_agg(environment ORDER BY environment)
           FROM "LiteLLM_PromptTable" WHERE prompt_id = $1"#,
        &[&"prompt-shared"],
    )
    .await;
    let environments: Vec<String> = row.get(0);
    assert_eq!(
        environments,
        vec!["development".to_owned(), "production".to_owned()]
    );

    let conflict = exec_params_err(
        client,
        r#"INSERT INTO "LiteLLM_PromptTable" (id, prompt_id, litellm_params, updated_at, environment)
           VALUES ($1, $2, '{}', now(), 'development')"#,
        &[&"prompt-dev-again", &"prompt-shared"],
    )
    .await;
    assert_eq!(sql_state(&conflict), SqlState::UNIQUE_VIOLATION.code());
}

async fn check_prompt_environment_repeat_42701(client: &Client) {
    batch_execute(
        client,
        migration_sql_of(ADD_PROMPT_ENVIRONMENT_AND_CREATED_BY),
    )
    .await;
    let error = batch_execute_err(
        client,
        migration_sql_of(ADD_PROMPT_ENVIRONMENT_AND_CREATED_BY),
    )
    .await;
    assert_eq!(sql_state(&error), SqlState::DUPLICATE_COLUMN.code());
}

async fn check_nullable_budget_limits(client: &Client) {
    assert_single_column(
        client,
        &["LiteLLM_VerificationToken", "LiteLLM_TeamTable"],
        ("budget_limits", "jsonb", true, None, "jsonb", None),
    )
    .await;
}

async fn check_member_allowed_models_empty_defaults(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ($1, $2, $3)"#,
        &[&"budget-a", &"tester", &"tester"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_TeamTable" (team_id) VALUES ($1)"#,
        &[&"team-a"],
    )
    .await;
    let allowed_models: Vec<String> = query_one_row(
        client,
        r#"SELECT allowed_models FROM "LiteLLM_BudgetTable" WHERE budget_id = $1"#,
        &[&"budget-a"],
    )
    .await
    .get(0);
    let default_team_member_models: Vec<String> = query_one_row(
        client,
        r#"SELECT default_team_member_models FROM "LiteLLM_TeamTable" WHERE team_id = $1"#,
        &[&"team-a"],
    )
    .await
    .get(0);
    assert!(allowed_models.is_empty());
    assert!(default_team_member_models.is_empty());
}

async fn check_nullable_mcp_instructions(client: &Client) {
    assert_column(
        client,
        "LiteLLM_MCPServerTable",
        ("instructions", "text", true, None, "text", None),
    )
    .await;
}

type BoxedCheck<'check> = Pin<Box<dyn Future<Output = ()> + Send + 'check>>;
type CheckFn = for<'check> fn(&'check Client) -> BoxedCheck<'check>;

fn boxed_byok_false_and_empty_metadata(client: &Client) -> BoxedCheck<'_> {
    Box::pin(check_byok_false_and_empty_metadata(client))
}

fn boxed_mcp_user_credential_raw_text_unique_pair(client: &Client) -> BoxedCheck<'_> {
    Box::pin(check_mcp_user_credential_raw_text_unique_pair(client))
}

fn boxed_jwt_mapping_active_default_and_unique_claim_pair(client: &Client) -> BoxedCheck<'_> {
    Box::pin(check_jwt_mapping_active_default_and_unique_claim_pair(
        client,
    ))
}

fn boxed_jwt_token_delete_restrict_update_cascade(client: &Client) -> BoxedCheck<'_> {
    Box::pin(check_jwt_token_delete_restrict_update_cascade(client))
}

fn boxed_config_override_required_json_and_updated_at(client: &Client) -> BoxedCheck<'_> {
    Box::pin(check_config_override_required_json_and_updated_at(client))
}

fn boxed_approval_fields_restore_preserves_values(client: &Client) -> BoxedCheck<'_> {
    Box::pin(check_approval_fields_restore_preserves_values(client))
}

fn boxed_team_lookup_indexes(client: &Client) -> BoxedCheck<'_> {
    Box::pin(check_team_lookup_indexes(client))
}

fn boxed_toolset_empty_tools_unique_name(client: &Client) -> BoxedCheck<'_> {
    Box::pin(check_toolset_empty_tools_unique_name(client))
}

fn boxed_prompt_development_backfill(client: &Client) -> BoxedCheck<'_> {
    Box::pin(check_prompt_development_backfill(client))
}

fn boxed_prompt_environment_distinct_versions(client: &Client) -> BoxedCheck<'_> {
    Box::pin(check_prompt_environment_distinct_versions(client))
}

fn boxed_prompt_environment_repeat_42701(client: &Client) -> BoxedCheck<'_> {
    Box::pin(check_prompt_environment_repeat_42701(client))
}

fn boxed_nullable_budget_limits(client: &Client) -> BoxedCheck<'_> {
    Box::pin(check_nullable_budget_limits(client))
}

fn boxed_member_allowed_models_empty_defaults(client: &Client) -> BoxedCheck<'_> {
    Box::pin(check_member_allowed_models_empty_defaults(client))
}

fn boxed_nullable_mcp_instructions(client: &Client) -> BoxedCheck<'_> {
    Box::pin(check_nullable_mcp_instructions(client))
}

#[rstest]
#[case::byok_false_and_empty_metadata(SCHEMA_SYNC_0306, boxed_byok_false_and_empty_metadata)]
#[case::mcp_user_credential_raw_text_unique_pair(
    SCHEMA_SYNC_0306,
    boxed_mcp_user_credential_raw_text_unique_pair
)]
#[case::jwt_mapping_active_default_and_unique_claim_pair(
    SCHEMA_SYNC_0306,
    boxed_jwt_mapping_active_default_and_unique_claim_pair
)]
#[case::jwt_token_delete_restrict_update_cascade(
    SCHEMA_SYNC_0306,
    boxed_jwt_token_delete_restrict_update_cascade
)]
#[case::config_override_required_json_and_updated_at(
    SCHEMA_SYNC_0306,
    boxed_config_override_required_json_and_updated_at
)]
#[case::approval_fields_restore_preserves_values(
    ADD_MCP_APPROVAL_STATUS,
    boxed_approval_fields_restore_preserves_values
)]
#[case::team_lookup_indexes(ADD_INDEX_TO_TEAM_TABLE, boxed_team_lookup_indexes)]
#[case::toolset_empty_tools_unique_name(ADD_MCP_TOOLSETS, boxed_toolset_empty_tools_unique_name)]
#[case::prompt_development_backfill(ADD_MCP_TOOLSETS, boxed_prompt_development_backfill)]
#[case::prompt_environment_distinct_versions(
    ADD_PROMPT_ENVIRONMENT_AND_CREATED_BY,
    boxed_prompt_environment_distinct_versions
)]
#[case::prompt_environment_repeat_42701(ADD_MCP_TOOLSETS, boxed_prompt_environment_repeat_42701)]
#[case::nullable_budget_limits(ADD_BUDGET_LIMITS, boxed_nullable_budget_limits)]
#[case::member_allowed_models_empty_defaults(
    ADD_TEAM_MEMBER_MODEL_SCOPE,
    boxed_member_allowed_models_empty_defaults
)]
#[case::nullable_mcp_instructions(ADD_MCP_SERVER_INSTRUCTIONS, boxed_nullable_mcp_instructions)]
#[tokio::test]
async fn postgres_mcp_and_prompt_fields(#[case] checkpoint: &'static str, #[case] check: CheckFn) {
    harness(move |[client]| async move {
        apply_migrations_through(&client, checkpoint).await;
        check(&client).await;
    })
    .await;
}
