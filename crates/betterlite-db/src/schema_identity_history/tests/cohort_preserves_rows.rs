use tokio_postgres::error::SqlState;

async fn count(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) -> i64 {
    query_one_row(client, sql, params).await.get(0)
}

async fn insert_pre_change_access_group(client: &Client) {
    let before_access_group_change = migration_sql_through("20260212143306_add_access_group_table");
    batch_execute(client, &before_access_group_change).await;

    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AccessGroupTable"
           (access_group_id, access_group_name, description, access_model_ids,
            access_mcp_server_ids, created_by, updated_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        &[
            &"ag1",
            &"group-one",
            &"desc",
            &vec!["m1".to_owned(), "m2".to_owned()],
            &vec!["s1".to_owned()],
            &"alice",
            &"alice",
        ],
    )
    .await;

    batch_execute(
        client,
        migration_sql_of("20260213170952_access_group_change_to_model_name"),
    )
    .await;
}

async fn assert_access_group_row_preserved(client: &Client) {
    assert!(!column_exists(client, "LiteLLM_AccessGroupTable", "access_model_ids").await);

    let name = scalar_text(
        client,
        r#"SELECT access_group_name FROM "LiteLLM_AccessGroupTable" WHERE access_group_id = 'ag1'"#,
        &[],
    )
    .await;
    assert_eq!(name, "group-one");
    let description = scalar_opt_text(
        client,
        r#"SELECT description FROM "LiteLLM_AccessGroupTable" WHERE access_group_id = 'ag1'"#,
        &[],
    )
    .await;
    assert_eq!(description, Some("desc".to_owned()));
    let model_names: Vec<String> = query_one_row(
        client,
        r#"SELECT access_model_names FROM "LiteLLM_AccessGroupTable" WHERE access_group_id = 'ag1'"#,
        &[],
    )
    .await
    .get(0);
    assert!(model_names.is_empty());
    let mcp_server_ids: Vec<String> = query_one_row(
        client,
        r#"SELECT access_mcp_server_ids FROM "LiteLLM_AccessGroupTable" WHERE access_group_id = 'ag1'"#,
        &[],
    )
    .await
    .get(0);
    assert_eq!(mcp_server_ids, vec!["s1".to_owned()]);
}

async fn assert_access_group_new_row_default(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AccessGroupTable" (access_group_id, access_group_name) VALUES ($1, $2)"#,
        &[&"ag2", &"group-two"],
    )
    .await;
    let new_model_names: Vec<String> = query_one_row(
        client,
        r#"SELECT access_model_names FROM "LiteLLM_AccessGroupTable" WHERE access_group_id = 'ag2'"#,
        &[],
    )
    .await
    .get(0);
    assert!(new_model_names.is_empty());
}

async fn assert_access_group_destructive_change(client: &Client) {
    insert_pre_change_access_group(client).await;
    assert_access_group_row_preserved(client).await;
    assert_access_group_new_row_default(client).await;
}

async fn assert_daily_endpoint_uniqueness(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DailyUserSpend"
           (id, user_id, date, api_key, model, custom_llm_provider,
            mcp_namespaced_tool_name, updated_at)
           VALUES ($1, 'u1', '2026-01-01', 'key1', 'gpt', 'openai', 'tool1', now())"#,
        &[&"null-endpoint-1"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DailyUserSpend"
           (id, user_id, date, api_key, model, custom_llm_provider,
            mcp_namespaced_tool_name, updated_at)
           VALUES ($1, 'u1', '2026-01-01', 'key1', 'gpt', 'openai', 'tool1', now())"#,
        &[&"null-endpoint-2"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DailyUserSpend"
           (id, user_id, date, api_key, model, custom_llm_provider,
            mcp_namespaced_tool_name, endpoint, updated_at)
           VALUES ($1, 'u1', '2026-01-01', 'key1', 'gpt', 'openai', 'tool1', 'chat/completions', now())"#,
        &[&"same-endpoint-1"],
    )
    .await;
    let conflict = exec_params_err(
        client,
        r#"INSERT INTO "LiteLLM_DailyUserSpend"
           (id, user_id, date, api_key, model, custom_llm_provider,
            mcp_namespaced_tool_name, endpoint, updated_at)
           VALUES ($1, 'u1', '2026-01-01', 'key1', 'gpt', 'openai', 'tool1', 'chat/completions', now())"#,
        &[&"same-endpoint-2"],
    )
    .await;
    assert_eq!(conflict.code(), Some(&SqlState::UNIQUE_VIOLATION));
}

async fn assert_end_user_fk_actions(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id) VALUES ($1)"#,
        &[&"op1"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_EndUserTable" (user_id, object_permission_id) VALUES ($1, $2)"#,
        &[&"eu1", &"op1"],
    )
    .await;

    exec_params(
        client,
        r#"UPDATE "LiteLLM_ObjectPermissionTable" SET object_permission_id = $1 WHERE object_permission_id = $2"#,
        &[&"op2", &"op1"],
    )
    .await;
    let cascaded = scalar_text(
        client,
        r#"SELECT object_permission_id FROM "LiteLLM_EndUserTable" WHERE user_id = 'eu1'"#,
        &[],
    )
    .await;
    assert_eq!(cascaded, "op2");

    exec_params(
        client,
        r#"DELETE FROM "LiteLLM_ObjectPermissionTable" WHERE object_permission_id = $1"#,
        &[&"op2"],
    )
    .await;
    let nulled = scalar_opt_text(
        client,
        r#"SELECT object_permission_id FROM "LiteLLM_EndUserTable" WHERE user_id = 'eu1'"#,
        &[],
    )
    .await;
    assert_eq!(nulled, None);
}

async fn assert_idempotent_reapplication(client: &Client) {
    let fk_count_before = count(
        client,
        "SELECT count(*) FROM pg_constraint WHERE conname = 'LiteLLM_EndUserTable_object_permission_id_fkey'",
        &[],
    )
    .await;
    assert_eq!(fk_count_before, 1);
    let column_count_before = columns_in(client, "LiteLLM_AccessGroupTable").await.len();

    batch_execute(client, &migration_sql_all()).await;

    let fk_count_after = count(
        client,
        "SELECT count(*) FROM pg_constraint WHERE conname = 'LiteLLM_EndUserTable_object_permission_id_fkey'",
        &[],
    )
    .await;
    assert_eq!(fk_count_after, 1);
    let column_count_after = columns_in(client, "LiteLLM_AccessGroupTable").await.len();
    assert_eq!(column_count_after, column_count_before);
}

#[tokio::test]
async fn cohort_preserves_rows() {
    harness(|[client]| async move {
        assert_access_group_destructive_change(&client).await;
        batch_execute(&client, &migration_sql_all()).await;
        assert_daily_endpoint_uniqueness(&client).await;
        assert_end_user_fk_actions(&client).await;
        assert_idempotent_reapplication(&client).await;
    })
    .await;
}
