const ADD_SPEND_LOG_TOOL_INDEX: &str = "20260226120000_add_spend_log_tool_index";
const ADD_AGENT_ID_TO_DELETE_KEYS: &str = "20260226202727_add_agent_id_to_delete_keys";
const ADD_CLAUDE_CODE_PLUGIN_TABLE: &str = "20260228000000_add_claude_code_plugin_table";
const ADD_SPEND_LOGS_COMPOSITE_INDEX: &str = "20260228100000_add_spend_logs_composite_index";
const MCP_DEFAULT_PUBLIC_INTERNET_TRUE: &str = "20260228110000_mcp_default_public_internet_true";
const SUPPORT_TEAM_BASED_GUARDRAILS: &str = "20260228170127_support_team_based_guardrails";
const UPDATE_TOOL_TABLE_POLICIES: &str = "20260303000000_update_tool_table_policies";
const ADD_RATE_LIMITS_TO_AGENTS: &str = "20260305000000_add_rate_limits_to_agents";
const SCHEMA_SYNC_20260312: &str = "20260312124619_schema_sync";

async fn assert_untrusted_default_text_column(client: &Client, table: &str, column: &'static str) {
    assert_column(
        client,
        table,
        (
            column,
            "text",
            false,
            Some("'untrusted'::text"),
            "text",
            None,
        ),
    )
    .await;
}

async fn check_untrusted_policy_and_zero_count(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ToolTable" (tool_id, tool_name) VALUES ($1, $2)"#,
        &[&"tool-a", &"tool-a-name"],
    )
    .await;
    assert_untrusted_default_text_column(client, "LiteLLM_ToolTable", "call_policy").await;
    assert_column(
        client,
        "LiteLLM_ToolTable",
        ("call_count", "integer", false, Some("0"), "int4", None),
    )
    .await;
    let policy = scalar_text(
        client,
        r#"SELECT call_policy FROM "LiteLLM_ToolTable" WHERE tool_id = $1"#,
        &[&"tool-a"],
    )
    .await;
    assert_eq!(policy, "untrusted");
}

async fn check_rename_input_policy_preserves_value(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ToolTable" (tool_id, tool_name, call_policy) VALUES ($1, $2, $3)"#,
        &[&"tool-b", &"tool-b-name", &"custom-input"],
    )
    .await;
    batch_execute(client, migration_sql_of(UPDATE_TOOL_TABLE_POLICIES)).await;
    assert!(!column_exists(client, "LiteLLM_ToolTable", "call_policy").await);
    let renamed = scalar_text(
        client,
        r#"SELECT input_policy FROM "LiteLLM_ToolTable" WHERE tool_id = $1"#,
        &[&"tool-b"],
    )
    .await;
    assert_eq!(renamed, "custom-input");
}

async fn check_output_untrusted_and_policy_indexes(client: &Client) {
    assert!(!index_exists(client, "LiteLLM_ToolTable_call_policy_idx").await);
    assert_table_catalog(
        client,
        "LiteLLM_ToolTable",
        &[(
            "output_policy",
            "text",
            false,
            Some("'untrusted'::text"),
            "text",
            None,
        )],
        &[
            (
                "LiteLLM_ToolTable_input_policy_idx",
                &["input_policy"],
                false,
            ),
            (
                "LiteLLM_ToolTable_output_policy_idx",
                &["output_policy"],
                false,
            ),
        ],
    )
    .await;
}

async fn check_tool_rename_guard_other_schema_failure(client: &Client) {
    batch_execute(client, migration_sql_of(UPDATE_TOOL_TABLE_POLICIES)).await;
    batch_execute(
        client,
        r#"CREATE SCHEMA other_schema; CREATE TABLE other_schema."LiteLLM_ToolTable" ("call_policy" TEXT);"#,
    )
    .await;
    let error = batch_execute_err(client, migration_sql_of(UPDATE_TOOL_TABLE_POLICIES)).await;
    assert_eq!(sql_state(&error), SqlState::UNDEFINED_COLUMN.code());
}

async fn check_blocked_tools_and_model_arrays(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id) VALUES ($1)"#,
        &[&"permission-a"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_ObjectPermissionTable",
        &[
            (
                "blocked_tools",
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
        ],
    )
    .await;
    let both_empty = scalar_bool(
        client,
        r#"SELECT blocked_tools = '{}' AND models = '{}' FROM "LiteLLM_ObjectPermissionTable" WHERE object_permission_id = $1"#,
        &[&"permission-a"],
    )
    .await;
    assert!(both_empty);
}

async fn check_spend_tool_composite_key(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_SpendLogToolIndex" (request_id, tool_name, start_time) VALUES ($1, $2, now())"#,
        &[&"req-a", &"tool-a"],
    )
    .await;
    let conflict = exec_params_err(
        client,
        r#"INSERT INTO "LiteLLM_SpendLogToolIndex" (request_id, tool_name, start_time) VALUES ($1, $2, now())"#,
        &[&"req-a", &"tool-a"],
    )
    .await;
    assert_eq!(sql_state(&conflict), SqlState::UNIQUE_VIOLATION.code());
}

async fn check_agent_permission_set_null_on_delete(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id) VALUES ($1)"#,
        &[&"permission-b"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AgentsTable"
           (agent_id, agent_name, agent_card_params, created_by, updated_by, object_permission_id)
           VALUES ($1, $2, '{}', 'tester', 'tester', $3)"#,
        &[&"agent-a", &"agent-a-name", &"permission-b"],
    )
    .await;
    exec_params(
        client,
        r#"DELETE FROM "LiteLLM_ObjectPermissionTable" WHERE object_permission_id = $1"#,
        &[&"permission-b"],
    )
    .await;
    let remaining: Option<String> = query_one_row(
        client,
        r#"SELECT object_permission_id FROM "LiteLLM_AgentsTable" WHERE agent_id = $1"#,
        &[&"agent-a"],
    )
    .await
    .get(0);
    assert_eq!(
        remaining, None,
        "ON DELETE SET NULL must clear the reference"
    );
}

async fn check_agent_permission_set_null_and_key_agent_nullable(client: &Client) {
    check_agent_permission_set_null_on_delete(client).await;

    assert_rule(
        client,
        "LiteLLM_AgentsTable_object_permission_id_fkey",
        'c',
        'n',
    )
    .await;

    assert_column(
        client,
        "LiteLLM_VerificationToken",
        ("agent_id", "text", true, None, "text", None),
    )
    .await;
    assert_column(
        client,
        "LiteLLM_DeletedVerificationToken",
        ("agent_id", "text", true, None, "text", None),
    )
    .await;
}

async fn check_plugin_text_files_and_enabled_true(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ClaudeCodePluginTable" (id, name) VALUES ($1, $2)"#,
        &[&"plugin-a", &"plugin-a-name"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_ClaudeCodePluginTable",
        ("files_json", "text", true, Some("'{}'::text"), "text", None),
    )
    .await;
    let enabled = scalar_bool(
        client,
        r#"SELECT enabled FROM "LiteLLM_ClaudeCodePluginTable" WHERE id = $1"#,
        &[&"plugin-a"],
    )
    .await;
    assert!(enabled);
}

async fn check_spend_time_request_index(client: &Client) {
    assert_index(
        client,
        "LiteLLM_SpendLogs_startTime_request_id_idx",
        &["startTime", "request_id"],
        false,
    )
    .await;
}

async fn check_public_internet_default_true_existing_false_unchanged(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, url, available_on_public_internet) VALUES ($1, $2, false)"#,
        &[&"server-existing", &"https://existing.test"],
    )
    .await;
    batch_execute(client, migration_sql_of(MCP_DEFAULT_PUBLIC_INTERNET_TRUE)).await;
    let existing = scalar_bool(
        client,
        r#"SELECT available_on_public_internet FROM "LiteLLM_MCPServerTable" WHERE server_id = $1"#,
        &[&"server-existing"],
    )
    .await;
    assert!(!existing, "existing rows keep their old value");

    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, url) VALUES ($1, $2)"#,
        &[&"server-fresh", &"https://fresh.test"],
    )
    .await;
    let fresh = scalar_bool(
        client,
        r#"SELECT available_on_public_internet FROM "LiteLLM_MCPServerTable" WHERE server_id = $1"#,
        &[&"server-fresh"],
    )
    .await;
    assert!(fresh, "new rows pick up the new default");
}

async fn check_guardrail_active_status_index(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_GuardrailsTable" (guardrail_id, guardrail_name, litellm_params, updated_at)
           VALUES ($1, $2, '{}', now())"#,
        &[&"guardrail-x", &"guardrail-x-name"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_GuardrailsTable",
        (
            "status",
            "text",
            false,
            Some("'active'::text"),
            "text",
            None,
        ),
    )
    .await;
    assert_index(
        client,
        "LiteLLM_GuardrailsTable_status_idx",
        &["status"],
        false,
    )
    .await;
    let status = scalar_text(
        client,
        r#"SELECT status FROM "LiteLLM_GuardrailsTable" WHERE guardrail_id = $1"#,
        &[&"guardrail-x"],
    )
    .await;
    assert_eq!(status, "active");
}

async fn assert_agent_spend_and_limit_columns(client: &Client) {
    assert_columns(
        client,
        "LiteLLM_AgentsTable",
        &[
            (
                "spend",
                "double precision",
                false,
                Some("0.0"),
                "float8",
                None,
            ),
            ("tpm_limit", "integer", true, None, "int4", None),
            ("rpm_limit", "integer", true, None, "int4", None),
            ("session_tpm_limit", "integer", true, None, "int4", None),
            ("session_rpm_limit", "integer", true, None, "int4", None),
            (
                "static_headers",
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

async fn check_agent_zero_spend_headers_and_four_integer_limits(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AgentsTable" (agent_id, agent_name, agent_card_params, created_by, updated_by)
           VALUES ($1, $2, '{}', 'tester', 'tester')"#,
        &[&"agent-limits", &"agent-limits-name"],
    )
    .await;
    assert_agent_spend_and_limit_columns(client).await;
    let row = query_one_row(
        client,
        r#"SELECT spend::text, static_headers::text, extra_headers, tpm_limit FROM "LiteLLM_AgentsTable" WHERE agent_id = $1"#,
        &[&"agent-limits"],
    )
    .await;
    let spend: String = row.get(0);
    let static_headers: String = row.get(1);
    let extra_headers: Vec<String> = row.get(2);
    let tpm_limit: Option<i32> = row.get(3);
    assert_eq!(spend, "0");
    assert_eq!(static_headers, "{}");
    assert!(extra_headers.is_empty());
    assert_eq!(tpm_limit, None);
}

#[derive(Clone, Copy)]
enum ToolAgentCohort {
    UntrustedPolicyAndZeroCount,
    RenameInputPolicyPreservesValue,
    OutputUntrustedAndPolicyIndexes,
    ToolRenameGuardOtherSchemaFailure,
    BlockedToolsAndModelArrays,
    SpendToolCompositeKey,
    AgentPermissionSetNullAndKeyAgentNullable,
    PluginTextFilesAndEnabledTrue,
    SpendTimeRequestIndex,
    PublicInternetDefaultTrueExistingFalseUnchanged,
    GuardrailActiveStatusIndex,
    AgentZeroSpendHeadersAndFourIntegerLimits,
}

fn tool_agent_checkpoint_for(cohort: ToolAgentCohort) -> &'static str {
    match cohort {
        ToolAgentCohort::UntrustedPolicyAndZeroCount
        | ToolAgentCohort::RenameInputPolicyPreservesValue
        | ToolAgentCohort::ToolRenameGuardOtherSchemaFailure
        | ToolAgentCohort::PublicInternetDefaultTrueExistingFalseUnchanged => {
            ADD_AGENT_OBJECT_PERMISSIONS_TABLE
        }
        ToolAgentCohort::AgentPermissionSetNullAndKeyAgentNullable => ADD_AGENT_ID_TO_DELETE_KEYS,
        ToolAgentCohort::OutputUntrustedAndPolicyIndexes => UPDATE_TOOL_TABLE_POLICIES,
        ToolAgentCohort::BlockedToolsAndModelArrays => SCHEMA_SYNC_20260312,
        ToolAgentCohort::SpendToolCompositeKey => ADD_SPEND_LOG_TOOL_INDEX,
        ToolAgentCohort::PluginTextFilesAndEnabledTrue => ADD_CLAUDE_CODE_PLUGIN_TABLE,
        ToolAgentCohort::SpendTimeRequestIndex => ADD_SPEND_LOGS_COMPOSITE_INDEX,
        ToolAgentCohort::GuardrailActiveStatusIndex => SUPPORT_TEAM_BASED_GUARDRAILS,
        ToolAgentCohort::AgentZeroSpendHeadersAndFourIntegerLimits => ADD_RATE_LIMITS_TO_AGENTS,
    }
}

async fn run_tool_agent_check(client: &Client, cohort: ToolAgentCohort) {
    match cohort {
        ToolAgentCohort::UntrustedPolicyAndZeroCount => {
            check_untrusted_policy_and_zero_count(client).await;
        }
        ToolAgentCohort::RenameInputPolicyPreservesValue => {
            check_rename_input_policy_preserves_value(client).await;
        }
        ToolAgentCohort::OutputUntrustedAndPolicyIndexes => {
            check_output_untrusted_and_policy_indexes(client).await;
        }
        ToolAgentCohort::ToolRenameGuardOtherSchemaFailure => {
            check_tool_rename_guard_other_schema_failure(client).await;
        }
        ToolAgentCohort::BlockedToolsAndModelArrays => {
            check_blocked_tools_and_model_arrays(client).await;
        }
        ToolAgentCohort::SpendToolCompositeKey => check_spend_tool_composite_key(client).await,
        ToolAgentCohort::AgentPermissionSetNullAndKeyAgentNullable => {
            check_agent_permission_set_null_and_key_agent_nullable(client).await;
        }
        ToolAgentCohort::PluginTextFilesAndEnabledTrue => {
            check_plugin_text_files_and_enabled_true(client).await;
        }
        ToolAgentCohort::SpendTimeRequestIndex => check_spend_time_request_index(client).await,
        ToolAgentCohort::PublicInternetDefaultTrueExistingFalseUnchanged => {
            check_public_internet_default_true_existing_false_unchanged(client).await;
        }
        ToolAgentCohort::GuardrailActiveStatusIndex => {
            check_guardrail_active_status_index(client).await;
        }
        ToolAgentCohort::AgentZeroSpendHeadersAndFourIntegerLimits => {
            check_agent_zero_spend_headers_and_four_integer_limits(client).await;
        }
    }
}

#[rstest]
#[case::untrusted_policy_and_zero_count(ToolAgentCohort::UntrustedPolicyAndZeroCount)]
#[case::rename_input_policy_preserves_value(ToolAgentCohort::RenameInputPolicyPreservesValue)]
#[case::output_untrusted_and_policy_indexes(ToolAgentCohort::OutputUntrustedAndPolicyIndexes)]
#[case::tool_rename_guard_other_schema_failure(ToolAgentCohort::ToolRenameGuardOtherSchemaFailure)]
#[case::blocked_tools_and_model_arrays(ToolAgentCohort::BlockedToolsAndModelArrays)]
#[case::spend_tool_composite_key(ToolAgentCohort::SpendToolCompositeKey)]
#[case::agent_permission_set_null_and_key_agent_nullable(
    ToolAgentCohort::AgentPermissionSetNullAndKeyAgentNullable
)]
#[case::plugin_text_files_and_enabled_true(ToolAgentCohort::PluginTextFilesAndEnabledTrue)]
#[case::spend_time_request_index(ToolAgentCohort::SpendTimeRequestIndex)]
#[case::public_internet_default_true_existing_false_unchanged(
    ToolAgentCohort::PublicInternetDefaultTrueExistingFalseUnchanged
)]
#[case::guardrail_active_status_index(ToolAgentCohort::GuardrailActiveStatusIndex)]
#[case::agent_zero_spend_headers_and_four_integer_limits(
    ToolAgentCohort::AgentZeroSpendHeadersAndFourIntegerLimits
)]
#[tokio::test]
async fn postgres_tool_agent_fields(#[case] cohort: ToolAgentCohort) {
    harness(move |[client]| async move {
        apply_migrations_through(&client, tool_agent_checkpoint_for(cohort)).await;
        run_tool_agent_check(&client, cohort).await;
    })
    .await;
}
