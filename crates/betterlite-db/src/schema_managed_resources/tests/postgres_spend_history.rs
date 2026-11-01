const TOOL_NAME_MIGRATION: &str = "20250707230009_add_mcp_namespaced_tool_name";

const COUNTER_COLUMNS: &[&str] = &[
    "prompt_tokens",
    "completion_tokens",
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
    "api_requests",
    "successful_requests",
    "failed_requests",
];

async fn assert_tool_name_index_replaced(client: &Client, table: &str, index: &str) {
    assert!(column_exists(client, table, "mcp_namespaced_tool_name").await);
    let facts = index_facts(client, index).await;
    assert_eq!(facts.columns, vec!["mcp_namespaced_tool_name"]);
    assert!(!facts.unique);
}

#[tokio::test]
async fn three_tool_indexes_replaced_in_order() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(TOOL_NAME_MIGRATION)).await;
        assert_tool_name_index_replaced(
            &client,
            "LiteLLM_DailyTagSpend",
            "LiteLLM_DailyTagSpend_mcp_namespaced_tool_name_idx",
        )
        .await;
        assert_tool_name_index_replaced(
            &client,
            "LiteLLM_DailyTeamSpend",
            "LiteLLM_DailyTeamSpend_mcp_namespaced_tool_name_idx",
        )
        .await;
        assert_tool_name_index_replaced(
            &client,
            "LiteLLM_DailyUserSpend",
            "LiteLLM_DailyUserSpend_mcp_namespaced_tool_name_idx",
        )
        .await;
    })
    .await;
}

async fn assert_old_index_name_has_six_columns(client: &Client, name: &str) {
    let facts = index_facts(client, name).await;
    assert_eq!(
        facts.columns.len(),
        6,
        "index {name} should carry 6 columns"
    );
    assert_eq!(
        facts.columns.last().unwrap(),
        "mcp_namespaced_tool_name",
        "index {name} should append the new tool-name column"
    );
    assert!(facts.unique, "index {name} must stay unique");
}

#[tokio::test]
async fn old_index_names_new_six_columns() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(TOOL_NAME_MIGRATION)).await;
        assert_old_index_name_has_six_columns(
            &client,
            "LiteLLM_DailyTagSpend_tag_date_api_key_model_custom_llm_pro_key",
        )
        .await;
        assert_old_index_name_has_six_columns(
            &client,
            "LiteLLM_DailyTeamSpend_team_id_date_api_key_model_custom_ll_key",
        )
        .await;
        assert_old_index_name_has_six_columns(
            &client,
            "LiteLLM_DailyUserSpend_user_id_date_api_key_model_custom_ll_key",
        )
        .await;
    })
    .await;
}

async fn assert_model_and_tool_name_nullable(client: &Client, table: &str) {
    assert!(column(client, table, "model").await.is_nullable);
    assert!(
        column(client, table, "mcp_namespaced_tool_name")
            .await
            .is_nullable
    );
}

#[tokio::test]
async fn nullable_models_and_tool_names() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(TOOL_NAME_MIGRATION)).await;
        assert_model_and_tool_name_nullable(&client, "LiteLLM_DailyTagSpend").await;
        assert_model_and_tool_name_nullable(&client, "LiteLLM_DailyTeamSpend").await;
        assert_model_and_tool_name_nullable(&client, "LiteLLM_DailyUserSpend").await;
        assert!(
            column(&client, "LiteLLM_SpendLogs", "mcp_namespaced_tool_name")
                .await
                .is_nullable
        );
    })
    .await;
}

async fn assert_counter_defaults(client: &Client, table: &str) {
    for counter in COUNTER_COLUMNS {
        let info = column(client, table, counter).await;
        assert!(!info.is_nullable, "{table}.{counter} must stay required");
        assert!(
            info.default.unwrap().contains('0'),
            "{table}.{counter} default"
        );
    }
    let spend = column(client, table, "spend").await;
    assert!(!spend.is_nullable);
    assert!(spend.default.unwrap().contains("0.0"));
}

#[tokio::test]
async fn organization_end_user_agent_counter_defaults() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_all()).await;
        assert_counter_defaults(&client, "LiteLLM_DailyOrganizationSpend").await;
        assert_counter_defaults(&client, "LiteLLM_DailyEndUserSpend").await;
        assert_counter_defaults(&client, "LiteLLM_DailyAgentSpend").await;
    })
    .await;
}

#[tokio::test]
async fn daily_entity_null_uniqueness() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_all()).await;
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_DailyOrganizationSpend\" \
             (id, organization_id, date, api_key, updated_at) \
             VALUES ($1, NULL, 'day-1', 'key-1', CURRENT_TIMESTAMP)",
            &[&"org-spend-1"],
        )
        .await;
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_DailyOrganizationSpend\" \
             (id, organization_id, date, api_key, updated_at) \
             VALUES ($1, NULL, 'day-1', 'key-1', CURRENT_TIMESTAMP)",
            &[&"org-spend-2"],
        )
        .await;
        let count = scalar_text(
            &client,
            "SELECT count(*)::TEXT FROM \"LiteLLM_DailyOrganizationSpend\" WHERE date = 'day-1'",
            &[],
        )
        .await;
        assert_eq!(count, "2");
    })
    .await;
}

#[tokio::test]
async fn tag_request_id() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20251114173537_add_request_id_to_daily_tag_spend"),
        )
        .await;
        let info = column(&client, "LiteLLM_DailyTagSpend", "request_id").await;
        assert!(info.is_nullable);
        assert_eq!(info.data_type, "text");
    })
    .await;
}

#[tokio::test]
async fn spend_organization_and_agent_nullable() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_all()).await;
        assert!(
            column(&client, "LiteLLM_SpendLogs", "organization_id")
                .await
                .is_nullable
        );
        assert!(
            column(&client, "LiteLLM_SpendLogs", "agent_id")
                .await
                .is_nullable
        );
    })
    .await;
}

#[tokio::test]
async fn repeated_agent_column_no_change() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_all()).await;
        let before = column(&client, "LiteLLM_SpendLogs", "agent_id").await;
        batch_execute(&client, migration_sql_of("20251211100212_schema_sync")).await;
        let after = column(&client, "LiteLLM_SpendLogs", "agent_id").await;
        assert_eq!(before.data_type, after.data_type);
        assert_eq!(before.is_nullable, after.is_nullable);
        assert_eq!(before.default, after.default);
    })
    .await;
}
