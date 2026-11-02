const BASELINE_DIFF: &str = "20260219181415_baseline_diff";
const SPEC_PATH_ADDED: &str = "20260220124742_add_spec_path_to_mcp_servers";
const COMPOSITE_INDEX_AGGREGATE_TABLES: &str =
    "20260220153844_add_composite_index_aggregate_tables";
const ENSURE_PROJECT_ID: &str = "20260221000000_ensure_project_id_verification_token";
const ADD_POLICY_VERSIONING: &str = "20260221183800_add_policy_versioning";
const ADD_BATCH_PROCESSED: &str = "20260222000000_add_batch_processed_to_managed_object_table";
const SPEND_LOGS_REQUEST_DURATION: &str = "20260224201417_spend_logs_request_duration";
const ADD_AGENT_OBJECT_PERMISSIONS_TABLE: &str =
    "20260224203854_add_agent_object_permissions_table";
const ADD_CONFIGS_OVERRIDE_TABLE: &str = "20260306175056_add_configs_override_table";

const ENTITY_DATE_COHORT: &[(&str, &str, &str, &str)] = &[
    (
        "LiteLLM_DailyAgentSpend",
        "agent_id",
        "LiteLLM_DailyAgentSpend_agent_id_idx",
        "LiteLLM_DailyAgentSpend_agent_id_date_idx",
    ),
    (
        "LiteLLM_DailyEndUserSpend",
        "end_user_id",
        "LiteLLM_DailyEndUserSpend_end_user_id_idx",
        "LiteLLM_DailyEndUserSpend_end_user_id_date_idx",
    ),
    (
        "LiteLLM_DailyOrganizationSpend",
        "organization_id",
        "LiteLLM_DailyOrganizationSpend_organization_id_idx",
        "LiteLLM_DailyOrganizationSpend_organization_id_date_idx",
    ),
    (
        "LiteLLM_DailyTagSpend",
        "tag",
        "LiteLLM_DailyTagSpend_tag_idx",
        "LiteLLM_DailyTagSpend_tag_date_idx",
    ),
    (
        "LiteLLM_DailyTeamSpend",
        "team_id",
        "LiteLLM_DailyTeamSpend_team_id_idx",
        "LiteLLM_DailyTeamSpend_team_id_date_idx",
    ),
    (
        "LiteLLM_DailyUserSpend",
        "user_id",
        "LiteLLM_DailyUserSpend_user_id_idx",
        "LiteLLM_DailyUserSpend_user_id_date_idx",
    ),
];

async fn insert_entity_date_row(client: &Client, table: &str, entity_column: &str, row_id: &str) {
    let sql = format!(
        r#"INSERT INTO "{table}" (id, "{entity_column}", date, api_key, updated_at)
           VALUES ($1, $2, '2026-02-20', 'sk-cohort', now())"#
    );
    exec_params(client, &sql, &[&row_id, &row_id]).await;
}

async fn assert_counter_default(client: &Client, table: &str, column_name: &str) {
    let info = column(client, table, column_name).await;
    assert!(
        !info.is_nullable,
        "{table}.{column_name} must stay required"
    );
    assert_eq!(info.data_type, "bigint");
    assert!(
        info.default.unwrap().contains('0'),
        "{table}.{column_name} default"
    );
}

async fn assert_metric_table_counter_defaults(client: &Client, table: &str) {
    for counter in [
        "requests_evaluated",
        "passed_count",
        "blocked_count",
        "flagged_count",
    ] {
        assert_counter_default(client, table, counter).await;
    }
}

async fn assert_metric_table_zero_counters_and_nullable_averages(client: &Client, table: &str) {
    assert_metric_table_counter_defaults(client, table).await;
    assert_columns(
        client,
        table,
        &[
            ("avg_score", "double precision", true, None, "float8", None),
            (
                "avg_latency_ms",
                "double precision",
                true,
                None,
                "float8",
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
}

async fn check_guardrail_policy_metric_zero_counters_and_nullable_averages(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DailyGuardrailMetrics" (guardrail_id, date, updated_at) VALUES ($1, $2, now())"#,
        &[&"guardrail-a", &"2026-02-19"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DailyPolicyMetrics" (policy_id, date, updated_at) VALUES ($1, $2, now())"#,
        &[&"policy-a", &"2026-02-19"],
    )
    .await;
    assert_metric_table_zero_counters_and_nullable_averages(
        client,
        "LiteLLM_DailyGuardrailMetrics",
    )
    .await;
    assert_metric_table_zero_counters_and_nullable_averages(client, "LiteLLM_DailyPolicyMetrics")
        .await;
}

async fn assert_spend_guardrail_index_catalog(client: &Client) {
    assert_table_catalog(
        client,
        "LiteLLM_SpendLogGuardrailIndex",
        &[
            ("request_id", "text", false, None, "text", None),
            ("guardrail_id", "text", false, None, "text", None),
            ("policy_id", "text", true, None, "text", None),
            (
                "start_time",
                "timestamp without time zone",
                false,
                None,
                "timestamp",
                Some(3_i32),
            ),
        ],
        &[
            (
                "LiteLLM_SpendLogGuardrailIndex_guardrail_id_start_time_idx",
                &["guardrail_id", "start_time"],
                false,
            ),
            (
                "LiteLLM_SpendLogGuardrailIndex_policy_id_start_time_idx",
                &["policy_id", "start_time"],
                false,
            ),
        ],
    )
    .await;
}

async fn check_spend_guardrail_index_pairs(client: &Client) {
    assert_spend_guardrail_index_catalog(client).await;
    assert_repeat_insert_unique_violation(
        client,
        r#"INSERT INTO "LiteLLM_SpendLogGuardrailIndex" (request_id, guardrail_id, start_time) VALUES ($1, $2, now())"#,
        &[&"req-a", &"guard-a"],
        &[&"req-a", &"guard-a"],
    )
    .await;
}

async fn check_six_entity_date_indexes(client: &Client) {
    for (table, entity_column, _old_index, _new_index) in ENTITY_DATE_COHORT.iter().copied() {
        insert_entity_date_row(client, table, entity_column, "row-before-migration").await;
    }
    batch_execute(client, migration_sql_of(COMPOSITE_INDEX_AGGREGATE_TABLES)).await;
    for (table, entity_column, old_index, new_index) in ENTITY_DATE_COHORT.iter().copied() {
        assert!(
            !index_exists(client, old_index).await,
            "{old_index} should have been dropped"
        );
        assert_index(client, new_index, &[entity_column, "date"], false).await;
        let remaining = scalar_text(
            client,
            &format!(r#"SELECT id FROM "{table}" WHERE id = 'row-before-migration'"#),
            &[],
        )
        .await;
        assert_eq!(remaining, "row-before-migration", "{table} data retained");
    }
}

async fn check_project_column_only_repair(client: &Client) {
    assert_column(
        client,
        "LiteLLM_VerificationToken",
        ("project_id", "text", true, None, "text", None),
    )
    .await;
    let foreign_keys = scalar_text(
        client,
        "SELECT count(*)::text FROM information_schema.key_column_usage k
           JOIN information_schema.table_constraints tc
             ON tc.constraint_name = k.constraint_name AND tc.table_schema = k.table_schema
           WHERE tc.constraint_type = 'FOREIGN KEY'
             AND k.table_name = 'LiteLLM_VerificationToken'
             AND k.column_name = 'project_id'",
        &[],
    )
    .await;
    assert_eq!(foreign_keys, "0", "no foreign key should be invented");
}

async fn check_policy_default_production_version_one(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_PolicyTable" (policy_id, policy_name) VALUES ($1, $2)"#,
        &[&"policy-existing", &"existing-policy"],
    )
    .await;
    batch_execute(client, migration_sql_of(ADD_POLICY_VERSIONING)).await;
    let row = query_one_row(
        client,
        r#"SELECT is_latest, version_number, version_status FROM "LiteLLM_PolicyTable" WHERE policy_id = $1"#,
        &[&"policy-existing"],
    )
    .await;
    let is_latest: bool = row.get(0);
    let version_number: i32 = row.get(1);
    let version_status: String = row.get(2);
    assert!(is_latest);
    assert_eq!(version_number, 1_i32);
    assert_eq!(version_status, "production");
}

async fn check_multiple_versions_same_latest_allowed(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_PolicyTable" (policy_id, policy_name, is_latest, version_number) VALUES ($1, $2, true, 1)"#,
        &[&"policy-v1", &"shared-name"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_PolicyTable" (policy_id, policy_name, is_latest, version_number) VALUES ($1, $2, true, 2)"#,
        &[&"policy-v2", &"shared-name"],
    )
    .await;
    let latest_count = scalar_text(
        client,
        r#"SELECT count(*)::text FROM "LiteLLM_PolicyTable" WHERE policy_name = $1 AND is_latest = true"#,
        &[&"shared-name"],
    )
    .await;
    assert_eq!(latest_count, "2");
}

async fn check_policy_version_duplicate_rejected(client: &Client) {
    assert_repeat_insert_unique_violation(
        client,
        r#"INSERT INTO "LiteLLM_PolicyTable" (policy_id, policy_name, version_number) VALUES ($1, $2, 5)"#,
        &[&"policy-first", &"duplicate-name"],
        &[&"policy-second", &"duplicate-name"],
    )
    .await;
}

async fn check_managed_batch_processed_false(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ManagedObjectTable"
           (id, unified_object_id, model_object_id, file_object, file_purpose, updated_at)
           VALUES ($1, $2, $3, '{}', 'batch', now())"#,
        &[&"object-a", &"unified-a", &"model-a"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_ManagedObjectTable",
        (
            "batch_processed",
            "boolean",
            false,
            Some("false"),
            "bool",
            None,
        ),
    )
    .await;
    let processed = scalar_bool(
        client,
        r#"SELECT batch_processed FROM "LiteLLM_ManagedObjectTable" WHERE id = $1"#,
        &[&"object-a"],
    )
    .await;
    assert!(!processed);
}

async fn check_nullable_request_duration(client: &Client) {
    assert_column(
        client,
        "LiteLLM_SpendLogs",
        ("request_duration_ms", "integer", true, None, "int4", None),
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, "startTime", "endTime")
           VALUES ($1, 'completion', now(), now())"#,
        &[&"req-duration"],
    )
    .await;
    let duration: Option<i32> = query_one_row(
        client,
        r#"SELECT request_duration_ms FROM "LiteLLM_SpendLogs" WHERE request_id = $1"#,
        &[&"req-duration"],
    )
    .await
    .get(0);
    assert_eq!(duration, None);
}

async fn check_spec_path_add_drop_readd_loses_old_values(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, url, spec_path) VALUES ($1, $2, $3)"#,
        &[&"server-a", &"https://example.test", &"original-spec"],
    )
    .await;
    batch_execute(client, migration_sql_of(ADD_AGENT_OBJECT_PERMISSIONS_TABLE)).await;
    assert!(!column_exists(client, "LiteLLM_MCPServerTable", "spec_path").await);

    batch_execute(client, migration_sql_of(ADD_CONFIGS_OVERRIDE_TABLE)).await;
    assert!(column_exists(client, "LiteLLM_MCPServerTable", "spec_path").await);
    let restored = scalar_opt_text(
        client,
        r#"SELECT spec_path FROM "LiteLLM_MCPServerTable" WHERE server_id = $1"#,
        &[&"server-a"],
    )
    .await;
    assert_eq!(
        restored, None,
        "the re-added column must not recover the dropped value"
    );
}

#[derive(Clone, Copy)]
enum PolicyHistoryCohort {
    GuardrailPolicyMetricZeroCountersAndNullableAverages,
    SpendGuardrailIndexPairs,
    SixEntityDateIndexes,
    ProjectColumnOnlyRepair,
    PolicyDefaultProductionVersionOne,
    MultipleVersionsSameLatestAllowed,
    PolicyVersionDuplicateRejected,
    ManagedBatchProcessedFalse,
    NullableRequestDuration,
    SpecPathAddDropReaddLosesOldValues,
}

fn policy_history_checkpoint_for(cohort: PolicyHistoryCohort) -> &'static str {
    match cohort {
        PolicyHistoryCohort::GuardrailPolicyMetricZeroCountersAndNullableAverages
        | PolicyHistoryCohort::SpendGuardrailIndexPairs => BASELINE_DIFF,
        PolicyHistoryCohort::SixEntityDateIndexes
        | PolicyHistoryCohort::SpecPathAddDropReaddLosesOldValues => SPEC_PATH_ADDED,
        PolicyHistoryCohort::ProjectColumnOnlyRepair
        | PolicyHistoryCohort::PolicyDefaultProductionVersionOne => ENSURE_PROJECT_ID,
        PolicyHistoryCohort::MultipleVersionsSameLatestAllowed
        | PolicyHistoryCohort::PolicyVersionDuplicateRejected => ADD_POLICY_VERSIONING,
        PolicyHistoryCohort::ManagedBatchProcessedFalse => ADD_BATCH_PROCESSED,
        PolicyHistoryCohort::NullableRequestDuration => SPEND_LOGS_REQUEST_DURATION,
    }
}

async fn run_policy_history_check(client: &Client, cohort: PolicyHistoryCohort) {
    match cohort {
        PolicyHistoryCohort::GuardrailPolicyMetricZeroCountersAndNullableAverages => {
            check_guardrail_policy_metric_zero_counters_and_nullable_averages(client).await;
        }
        PolicyHistoryCohort::SpendGuardrailIndexPairs => {
            check_spend_guardrail_index_pairs(client).await;
        }
        PolicyHistoryCohort::SixEntityDateIndexes => check_six_entity_date_indexes(client).await,
        PolicyHistoryCohort::ProjectColumnOnlyRepair => {
            check_project_column_only_repair(client).await;
        }
        PolicyHistoryCohort::PolicyDefaultProductionVersionOne => {
            check_policy_default_production_version_one(client).await;
        }
        PolicyHistoryCohort::MultipleVersionsSameLatestAllowed => {
            check_multiple_versions_same_latest_allowed(client).await;
        }
        PolicyHistoryCohort::PolicyVersionDuplicateRejected => {
            check_policy_version_duplicate_rejected(client).await;
        }
        PolicyHistoryCohort::ManagedBatchProcessedFalse => {
            check_managed_batch_processed_false(client).await;
        }
        PolicyHistoryCohort::NullableRequestDuration => {
            check_nullable_request_duration(client).await;
        }
        PolicyHistoryCohort::SpecPathAddDropReaddLosesOldValues => {
            check_spec_path_add_drop_readd_loses_old_values(client).await;
        }
    }
}

#[rstest]
#[case::guardrail_policy_metric_zero_counters_and_nullable_averages(
    PolicyHistoryCohort::GuardrailPolicyMetricZeroCountersAndNullableAverages
)]
#[case::spend_guardrail_index_pairs(PolicyHistoryCohort::SpendGuardrailIndexPairs)]
#[case::six_entity_date_indexes(PolicyHistoryCohort::SixEntityDateIndexes)]
#[case::project_column_only_repair(PolicyHistoryCohort::ProjectColumnOnlyRepair)]
#[case::policy_default_production_version_one(
    PolicyHistoryCohort::PolicyDefaultProductionVersionOne
)]
#[case::multiple_versions_same_latest_allowed(
    PolicyHistoryCohort::MultipleVersionsSameLatestAllowed
)]
#[case::policy_version_duplicate_rejected(PolicyHistoryCohort::PolicyVersionDuplicateRejected)]
#[case::managed_batch_processed_false(PolicyHistoryCohort::ManagedBatchProcessedFalse)]
#[case::nullable_request_duration(PolicyHistoryCohort::NullableRequestDuration)]
#[case::spec_path_add_drop_readd_loses_old_values(
    PolicyHistoryCohort::SpecPathAddDropReaddLosesOldValues
)]
#[tokio::test]
async fn postgres_policy_history(#[case] cohort: PolicyHistoryCohort) {
    harness(move |[client]| async move {
        apply_migrations_through(&client, policy_history_checkpoint_for(cohort)).await;
        run_policy_history_check(&client, cohort).await;
    })
    .await;
}
