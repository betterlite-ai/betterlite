#[derive(Clone, Copy)]
enum BudgetAndUsageCohort {
    HeartbeatTimestampDefaults,
    GuardrailUnitsZeroCostNullUntrackedZero,
    SpendLogTimestampBackfill,
    RequestIndexMigrationsAreNoops,
    SixGatewaySavingsAndTimingDefaults,
    AccessGroupBudgetSetNull,
    ManagedOrgNoBackfill,
    McpDiscoveryFalse,
    SkillAgentEmptyArrays,
    FiveNullableTpdLimits,
    TemporaryBudgetFieldsNull,
    KeyTotalSpendZeroNotCurrentSpend,
    PolicyPriorityNullDefaultFlagFalse,
    PasswordResetAndBreachTimeNull,
    ManagedContentRequiredBytea,
}

fn budget_and_usage_checkpoint_for(cohort: BudgetAndUsageCohort) -> &'static str {
    match cohort {
        BudgetAndUsageCohort::HeartbeatTimestampDefaults => ADD_PROXY_WORKER_HEARTBEAT.directory,
        BudgetAndUsageCohort::GuardrailUnitsZeroCostNullUntrackedZero => {
            ADD_GUARDRAIL_USAGE_UNITS_COST.directory
        }
        BudgetAndUsageCohort::SpendLogTimestampBackfill => {
            ADD_DAILY_GUARDRAIL_USAGE_UNITS.directory
        }
        BudgetAndUsageCohort::RequestIndexMigrationsAreNoops => SHADOW_EVAL_MAX_BUDGET.directory,
        BudgetAndUsageCohort::SixGatewaySavingsAndTimingDefaults => {
            ADD_DAILY_RESPONSE_TIME.directory
        }
        BudgetAndUsageCohort::AccessGroupBudgetSetNull => {
            ADD_MODEL_ACCESS_GROUP_BUDGET_TABLE.directory
        }
        BudgetAndUsageCohort::ManagedOrgNoBackfill => ADD_GUARDRAIL_USAGE_UNITS_COST.directory,
        BudgetAndUsageCohort::McpDiscoveryFalse => {
            ADD_PER_SERVER_OAUTH_DISCOVERY_TO_MCP_SERVERS.directory
        }
        BudgetAndUsageCohort::SkillAgentEmptyArrays => ADD_AGENT_ACCESS_GROUP_IDS.directory,
        BudgetAndUsageCohort::FiveNullableTpdLimits => ADD_TPD_LIMIT.directory,
        BudgetAndUsageCohort::TemporaryBudgetFieldsNull => ADD_BUDGET_TEMP_INCREASE.directory,
        BudgetAndUsageCohort::KeyTotalSpendZeroNotCurrentSpend => ADD_KEY_TOTAL_SPEND.directory,
        BudgetAndUsageCohort::PolicyPriorityNullDefaultFlagFalse => {
            ADD_POLICY_ATTACHMENT_IS_DEFAULT.directory
        }
        BudgetAndUsageCohort::PasswordResetAndBreachTimeNull => {
            ADD_PASSWORD_RESET_COLUMNS.directory
        }
        BudgetAndUsageCohort::ManagedContentRequiredBytea => {
            ADD_MANAGED_FILE_CONTENT_TABLE.directory
        }
    }
}

fn timestamp3_now_default(name: &'static str) -> ColumnSpec {
    (
        name,
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
        "timestamp",
        Some(3),
    )
}

async fn check_heartbeat_timestamp_defaults(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ProxyWorkerHeartbeat" (worker_id, hostname) VALUES ($1, $2)"#,
        &[&"worker-a", &"host-a"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_ProxyWorkerHeartbeat",
        &[
            timestamp3_now_default("started_at"),
            timestamp3_now_default("last_heartbeat_at"),
        ],
    )
    .await;
}

async fn check_guardrail_units_zero_cost_null_untracked_zero(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DailyGuardrailUsageUnits" (guardrail_id, date, team_id, api_key, usage_unit, updated_at) VALUES ($1, $2, $3, $4, $5, now())"#,
        &[&"guardrail-a", &"2026-01-01", &"team-a", &"key-a", &"requests"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_DailyGuardrailUsageUnits",
        &[
            int8_zero("units"),
            ("cost", "double precision", true, None, "float8", None),
            int8_zero("untracked_units"),
        ],
    )
    .await;
}

async fn check_spend_log_timestamp_backfill(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, "startTime", "endTime") VALUES ($1, $2, now(), now())"#,
        &[&"request-a", &"completion"],
    )
    .await;
    batch_execute(client, migration_sql_of(ADD_SPEND_LOG_TIMESTAMPS.directory)).await;
    let row = query_one_row(
        client,
        r#"SELECT created_at, updated_at FROM "LiteLLM_SpendLogs" WHERE request_id = $1"#,
        &[&"request-a"],
    )
    .await;
    let created_at: Option<SystemTime> = row.get(0);
    let updated_at: Option<SystemTime> = row.get(1);
    assert!(created_at.is_some());
    assert!(updated_at.is_some());
}

async fn check_request_index_migrations_are_noops(client: &Client) {
    let before_first = catalog_snapshot(client).await;
    batch_execute(
        client,
        migration_sql_of(ADD_SPEND_LOGS_API_KEY_STARTTIME_INDEX.directory),
    )
    .await;
    assert_eq!(before_first, catalog_snapshot(client).await);
    for directory in [
        ADD_GATEWAY_INJECTED_CACHING_SAVINGS_SPEND.directory,
        SHADOW_EVAL_COST_COMPARISON.directory,
        ADD_MODEL_ACCESS_GROUP_BUDGET_TABLE.directory,
        SHADOW_EVAL_TYPED_TARGETS.directory,
        ADD_LITELLM_CALL_ID_SPEND_LOGS.directory,
    ] {
        batch_execute(client, migration_sql_of(directory)).await;
    }
    let before_second = catalog_snapshot(client).await;
    batch_execute(
        client,
        migration_sql_of(SPEND_LOGS_LITELLM_CALL_ID_INDEX.directory),
    )
    .await;
    assert_eq!(before_second, catalog_snapshot(client).await);
}

async fn seed_six_daily_spend_tables(client: &Client) {
    batch_execute(
        client,
        r#"INSERT INTO "LiteLLM_DailyUserSpend" (id, user_id, model, date, api_key, updated_at) VALUES ('spend-user', 'user-a', 'model-a', 'date-a', 'key-a', now())"#,
    )
    .await;
    batch_execute(
        client,
        r#"INSERT INTO "LiteLLM_DailyTeamSpend" (id, team_id, model, date, api_key, updated_at) VALUES ('spend-team', 'team-a', 'model-a', 'date-a', 'key-a', now())"#,
    )
    .await;
    batch_execute(
        client,
        r#"INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, model, date, api_key, updated_at) VALUES ('spend-tag', 'tag-a', 'model-a', 'date-a', 'key-a', now())"#,
    )
    .await;
    batch_execute(
        client,
        r#"INSERT INTO "LiteLLM_DailyOrganizationSpend" (id, date, api_key, updated_at) VALUES ('spend-org', 'date-a', 'key-a', now())"#,
    )
    .await;
    batch_execute(
        client,
        r#"INSERT INTO "LiteLLM_DailyEndUserSpend" (id, date, api_key, updated_at) VALUES ('spend-end-user', 'date-a', 'key-a', now())"#,
    )
    .await;
    batch_execute(
        client,
        r#"INSERT INTO "LiteLLM_DailyAgentSpend" (id, date, api_key, updated_at) VALUES ('spend-agent', 'date-a', 'key-a', now())"#,
    )
    .await;
}

async fn check_six_gateway_savings_and_timing_defaults(client: &Client) {
    seed_six_daily_spend_tables(client).await;
    for table in [
        "LiteLLM_DailyUserSpend",
        "LiteLLM_DailyOrganizationSpend",
        "LiteLLM_DailyEndUserSpend",
        "LiteLLM_DailyAgentSpend",
        "LiteLLM_DailyTeamSpend",
        "LiteLLM_DailyTagSpend",
    ] {
        assert_columns(
            client,
            table,
            &[
                float8_default("gateway_injected_caching_savings_spend", "0.0"),
                int8_zero("total_response_time_ms"),
                int8_zero("timed_requests"),
            ],
        )
        .await;
    }
}

async fn check_access_group_budget_set_null(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ($1, $2, $2)"#,
        &[&"budget-a", &"operator"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ModelAccessGroupBudgetTable" (access_group_name, budget_id) VALUES ($1, $2)"#,
        &[&"group-a", &"budget-a"],
    )
    .await;
    assert_rule(
        client,
        "LiteLLM_ModelAccessGroupBudgetTable_budget_id_fkey",
        'c',
        'n',
    )
    .await;
    exec_params(
        client,
        r#"DELETE FROM "LiteLLM_BudgetTable" WHERE budget_id = $1"#,
        &[&"budget-a"],
    )
    .await;
    let budget_id = scalar_opt_text(
        client,
        r#"SELECT budget_id FROM "LiteLLM_ModelAccessGroupBudgetTable" WHERE access_group_name = $1"#,
        &[&"group-a"],
    )
    .await;
    assert_eq!(budget_id, None);
}

async fn check_managed_org_no_backfill(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ManagedObjectTable" (id, unified_object_id, model_object_id, file_object, file_purpose, updated_at) VALUES ($1, $2, $3, '{}', 'batch', now())"#,
        &[&"object-org", &"unified-org", &"model-object-org"],
    )
    .await;
    batch_execute(
        client,
        migration_sql_of(ADD_ORG_ID_TO_MANAGED_OBJECT_TABLE.directory),
    )
    .await;
    let org_id = scalar_opt_text(
        client,
        r#"SELECT org_id FROM "LiteLLM_ManagedObjectTable" WHERE id = $1"#,
        &[&"object-org"],
    )
    .await;
    assert_eq!(org_id, None);
}

async fn check_mcp_discovery_false(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, url) VALUES ($1, $2)"#,
        &[&"server-a", &"https://example.test/mcp"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_MCPServerTable",
        bool_false("per_server_oauth_discovery"),
    )
    .await;
}

async fn check_skill_agent_empty_arrays(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id) VALUES ($1)"#,
        &[&"permission-a"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_ObjectPermissionTable",
        empty_text_array("skills"),
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AgentsTable" (agent_id, agent_name, agent_card_params, created_by, updated_by) VALUES ($1, $2, '{}', $3, $3)"#,
        &[&"agent-a", &"agent-name-a", &"operator"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_AgentsTable",
        empty_text_array("access_group_ids"),
    )
    .await;
}

async fn check_five_nullable_tpd_limits(client: &Client) {
    batch_execute(
        client,
        r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('budget-tpd', 'operator', 'operator')"#,
    )
    .await;
    batch_execute(
        client,
        r#"INSERT INTO "LiteLLM_TeamTable" (team_id) VALUES ('team-tpd')"#,
    )
    .await;
    batch_execute(
        client,
        r#"INSERT INTO "LiteLLM_DeletedTeamTable" (id, team_id) VALUES ('deleted-team-tpd', 'team-tpd')"#,
    )
    .await;
    batch_execute(
        client,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token) VALUES ('token-tpd')"#,
    )
    .await;
    batch_execute(
        client,
        r#"INSERT INTO "LiteLLM_DeletedVerificationToken" (id, token) VALUES ('deleted-token-tpd', 'token-tpd')"#,
    )
    .await;
    assert_single_column(
        client,
        &[
            "LiteLLM_BudgetTable",
            "LiteLLM_TeamTable",
            "LiteLLM_DeletedTeamTable",
            "LiteLLM_VerificationToken",
            "LiteLLM_DeletedVerificationToken",
        ],
        ("tpd_limit", "bigint", true, None, "int8", None),
    )
    .await;
}

async fn check_temporary_budget_fields_null(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ($1, $2, $2)"#,
        &[&"budget-temp", &"operator"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_BudgetTable",
        &[
            (
                "temp_budget_increase",
                "double precision",
                true,
                None,
                "float8",
                None,
            ),
            (
                "temp_budget_expiry",
                "timestamp without time zone",
                true,
                None,
                "timestamp",
                Some(3_i32),
            ),
        ],
    )
    .await;
}

async fn check_key_total_spend_zero_not_current_spend(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token, spend) VALUES ($1, $2)"#,
        &[&"token-total", &42.5_f64],
    )
    .await;
    let row = query_one_row(
        client,
        r#"SELECT spend::text, total_spend::text FROM "LiteLLM_VerificationToken" WHERE token = $1"#,
        &[&"token-total"],
    )
    .await;
    let spend: String = row.get(0);
    let total_spend: String = row.get(1);
    assert_eq!(spend, "42.5");
    assert_eq!(total_spend, "0");
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DeletedVerificationToken" (id, token, spend) VALUES ($1, $2, $3)"#,
        &[&"deleted-token-total", &"token-total-2", &7.0_f64],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_DeletedVerificationToken",
        float8_default("total_spend", "0.0"),
    )
    .await;
}

async fn check_policy_priority_null_default_flag_false(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_PolicyAttachmentTable" (attachment_id, policy_name) VALUES ($1, $2)"#,
        &[&"attachment-a", &"policy-a"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_PolicyAttachmentTable",
        &[
            ("priority", "integer", true, None, "int4", None),
            bool_false("is_default"),
        ],
    )
    .await;
    let is_default = scalar_bool(
        client,
        r#"SELECT is_default FROM "LiteLLM_PolicyAttachmentTable" WHERE attachment_id = $1"#,
        &[&"attachment-a"],
    )
    .await;
    assert!(!is_default);
}

async fn check_password_reset_and_breach_time_null(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ($1)"#,
        &[&"user-reset"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_UserTable",
        &[
            nullable_bool("password_reset_required"),
            (
                "last_breach_check_at",
                "timestamp without time zone",
                true,
                None,
                "timestamp",
                Some(3_i32),
            ),
        ],
    )
    .await;
}

async fn check_managed_content_required_bytea(client: &Client) {
    assert_exec_params_err_state(
        client,
        r#"INSERT INTO "LiteLLM_ManagedFileContentTable" (id) VALUES ($1)"#,
        &[&"content-missing"],
        SqlState::NOT_NULL_VIOLATION,
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ManagedFileContentTable" (id, content) VALUES ($1, $2)"#,
        &[&"content-a", &b"payload".as_slice()],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_ManagedFileContentTable",
        ("content", "bytea", false, None, "bytea", None),
    )
    .await;
}

async fn run_budget_and_usage_check_early(client: &Client, cohort: BudgetAndUsageCohort) -> bool {
    match cohort {
        BudgetAndUsageCohort::HeartbeatTimestampDefaults => {
            check_heartbeat_timestamp_defaults(client).await;
        }
        BudgetAndUsageCohort::GuardrailUnitsZeroCostNullUntrackedZero => {
            check_guardrail_units_zero_cost_null_untracked_zero(client).await;
        }
        BudgetAndUsageCohort::SpendLogTimestampBackfill => {
            check_spend_log_timestamp_backfill(client).await;
        }
        BudgetAndUsageCohort::RequestIndexMigrationsAreNoops => {
            check_request_index_migrations_are_noops(client).await;
        }
        BudgetAndUsageCohort::SixGatewaySavingsAndTimingDefaults => {
            check_six_gateway_savings_and_timing_defaults(client).await;
        }
        BudgetAndUsageCohort::AccessGroupBudgetSetNull => {
            check_access_group_budget_set_null(client).await;
        }
        BudgetAndUsageCohort::ManagedOrgNoBackfill => {
            check_managed_org_no_backfill(client).await;
        }
        BudgetAndUsageCohort::McpDiscoveryFalse => {
            check_mcp_discovery_false(client).await;
        }
        BudgetAndUsageCohort::SkillAgentEmptyArrays
        | BudgetAndUsageCohort::FiveNullableTpdLimits
        | BudgetAndUsageCohort::TemporaryBudgetFieldsNull
        | BudgetAndUsageCohort::KeyTotalSpendZeroNotCurrentSpend
        | BudgetAndUsageCohort::PolicyPriorityNullDefaultFlagFalse
        | BudgetAndUsageCohort::PasswordResetAndBreachTimeNull
        | BudgetAndUsageCohort::ManagedContentRequiredBytea => return false,
    }
    true
}

async fn run_budget_and_usage_check_late(client: &Client, cohort: BudgetAndUsageCohort) {
    match cohort {
        BudgetAndUsageCohort::SkillAgentEmptyArrays => {
            check_skill_agent_empty_arrays(client).await;
        }
        BudgetAndUsageCohort::FiveNullableTpdLimits => {
            check_five_nullable_tpd_limits(client).await;
        }
        BudgetAndUsageCohort::TemporaryBudgetFieldsNull => {
            check_temporary_budget_fields_null(client).await;
        }
        BudgetAndUsageCohort::KeyTotalSpendZeroNotCurrentSpend => {
            check_key_total_spend_zero_not_current_spend(client).await;
        }
        BudgetAndUsageCohort::PolicyPriorityNullDefaultFlagFalse => {
            check_policy_priority_null_default_flag_false(client).await;
        }
        BudgetAndUsageCohort::PasswordResetAndBreachTimeNull => {
            check_password_reset_and_breach_time_null(client).await;
        }
        BudgetAndUsageCohort::ManagedContentRequiredBytea => {
            check_managed_content_required_bytea(client).await;
        }
        BudgetAndUsageCohort::HeartbeatTimestampDefaults
        | BudgetAndUsageCohort::GuardrailUnitsZeroCostNullUntrackedZero
        | BudgetAndUsageCohort::SpendLogTimestampBackfill
        | BudgetAndUsageCohort::RequestIndexMigrationsAreNoops
        | BudgetAndUsageCohort::SixGatewaySavingsAndTimingDefaults
        | BudgetAndUsageCohort::AccessGroupBudgetSetNull
        | BudgetAndUsageCohort::ManagedOrgNoBackfill
        | BudgetAndUsageCohort::McpDiscoveryFalse => {}
    }
}

async fn run_budget_and_usage_check(client: &Client, cohort: BudgetAndUsageCohort) {
    if run_budget_and_usage_check_early(client, cohort).await {
        return;
    }
    run_budget_and_usage_check_late(client, cohort).await;
}

#[rstest]
#[case::heartbeat_timestamp_defaults(BudgetAndUsageCohort::HeartbeatTimestampDefaults)]
#[case::guardrail_units_zero_cost_null_untracked_zero(
    BudgetAndUsageCohort::GuardrailUnitsZeroCostNullUntrackedZero
)]
#[case::spend_log_timestamp_backfill(BudgetAndUsageCohort::SpendLogTimestampBackfill)]
#[case::request_index_migrations_are_noops(BudgetAndUsageCohort::RequestIndexMigrationsAreNoops)]
#[case::six_gateway_savings_and_timing_defaults(
    BudgetAndUsageCohort::SixGatewaySavingsAndTimingDefaults
)]
#[case::access_group_budget_set_null(BudgetAndUsageCohort::AccessGroupBudgetSetNull)]
#[case::managed_org_no_backfill(BudgetAndUsageCohort::ManagedOrgNoBackfill)]
#[case::mcp_discovery_false(BudgetAndUsageCohort::McpDiscoveryFalse)]
#[case::skill_agent_empty_arrays(BudgetAndUsageCohort::SkillAgentEmptyArrays)]
#[case::five_nullable_tpd_limits(BudgetAndUsageCohort::FiveNullableTpdLimits)]
#[case::temporary_budget_fields_null(BudgetAndUsageCohort::TemporaryBudgetFieldsNull)]
#[case::key_total_spend_zero_not_current_spend(
    BudgetAndUsageCohort::KeyTotalSpendZeroNotCurrentSpend
)]
#[case::policy_priority_null_default_flag_false(
    BudgetAndUsageCohort::PolicyPriorityNullDefaultFlagFalse
)]
#[case::password_reset_and_breach_time_null(BudgetAndUsageCohort::PasswordResetAndBreachTimeNull)]
#[case::managed_content_required_bytea(BudgetAndUsageCohort::ManagedContentRequiredBytea)]
#[tokio::test]
async fn postgres_budget_and_usage(#[case] cohort: BudgetAndUsageCohort) {
    Box::pin(harness(move |[client]| async move {
        apply_migrations_through(&client, budget_and_usage_checkpoint_for(cohort)).await;
        run_budget_and_usage_check(&client, cohort).await;
    }))
    .await;
}
