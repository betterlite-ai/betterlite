async fn seed_mcp_server(client: &Client, server_id: &str, alias: Option<&str>) {
    exec_params(
        client,
        "INSERT INTO \"LiteLLM_MCPServerTable\" (server_id, alias, url) VALUES ($1, $2, $3)",
        &[&server_id, &alias, &"https://example.com"],
    )
    .await;
}

#[tokio::test]
async fn stdio_arrays_json_and_nullable_url() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20250711220620_add_stdio_mcp"),
        )
        .await;
        let args = column(&client, "LiteLLM_MCPServerTable", "args").await;
        assert!(args.is_nullable);
        assert!(args.default.unwrap().contains("ARRAY"));
        let command = column(&client, "LiteLLM_MCPServerTable", "command").await;
        assert!(command.is_nullable);
        assert!(command.default.is_none());
        let env = column(&client, "LiteLLM_MCPServerTable", "env").await;
        assert!(env.is_nullable);
        assert!(env.default.unwrap().contains("{}"));
        assert!(
            column(&client, "LiteLLM_MCPServerTable", "url")
                .await
                .is_nullable
        );
    })
    .await;
}

#[tokio::test]
async fn server_groups_no_default_permission_groups_empty() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20250711220620_add_stdio_mcp"),
        )
        .await;
        let server_groups = column(&client, "LiteLLM_MCPServerTable", "mcp_access_groups").await;
        assert!(server_groups.is_nullable);
        assert!(server_groups.default.is_none());
        let permission_groups = column(
            &client,
            "LiteLLM_ObjectPermissionTable",
            "mcp_access_groups",
        )
        .await;
        assert!(permission_groups.is_nullable);
        assert!(permission_groups.default.unwrap().contains("ARRAY"));
    })
    .await;
}

#[tokio::test]
async fn alias_values_move_new_alias_null() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20250802162330_prompt_table"),
        )
        .await;
        seed_mcp_server(&client, "server-1", Some("old-alias")).await;
        batch_execute(
            &client,
            migration_sql_of("20250806095134_rename_alias_to_server_name_mcp_table"),
        )
        .await;
        let server_name = scalar_text(
            &client,
            "SELECT server_name FROM \"LiteLLM_MCPServerTable\" WHERE server_id = $1",
            &[&"server-1"],
        )
        .await;
        assert_eq!(server_name, "old-alias");
        let alias = scalar_opt_text(
            &client,
            "SELECT alias FROM \"LiteLLM_MCPServerTable\" WHERE server_id = $1",
            &[&"server-1"],
        )
        .await;
        assert_eq!(alias, None);
    })
    .await;
}

#[tokio::test]
async fn rename_repeat_duplicate_column_42701() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20250806095134_rename_alias_to_server_name_mcp_table"),
        )
        .await;
        let error = batch_execute_err(
            &client,
            migration_sql_of("20250806095134_rename_alias_to_server_name_mcp_table"),
        )
        .await;
        assert_eq!(error.code().unwrap().code(), "42701");
    })
    .await;
}

#[tokio::test]
async fn other_schema_alias_undefined_column_42703() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20250802162330_prompt_table"),
        )
        .await;
        batch_execute(
            &client,
            "ALTER TABLE \"LiteLLM_MCPServerTable\" DROP COLUMN IF EXISTS \"alias\"; \
             CREATE SCHEMA \"other_schema\"; \
             CREATE TABLE \"other_schema\".\"LiteLLM_MCPServerTable\" (\"alias\" TEXT);",
        )
        .await;
        let error = batch_execute_err(
            &client,
            migration_sql_of("20250806095134_rename_alias_to_server_name_mcp_table"),
        )
        .await;
        assert_eq!(error.code().unwrap().code(), "42703");
    })
    .await;
}

#[tokio::test]
async fn spec_version_drop_loses_only_that_column() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20250918083359_drop_spec_version_column_from_mcp_table"),
        )
        .await;
        assert!(!column_exists(&client, "LiteLLM_MCPServerTable", "spec_version").await);
        assert!(column_exists(&client, "LiteLLM_MCPServerTable", "transport").await);
        assert!(column_exists(&client, "LiteLLM_MCPServerTable", "url").await);
    })
    .await;
}

#[tokio::test]
async fn allowed_tools_extra_headers_empty() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20251003190954_extra_headers_to_mcp_table"),
        )
        .await;
        let allowed_tools = column(&client, "LiteLLM_MCPServerTable", "allowed_tools").await;
        assert!(allowed_tools.is_nullable);
        assert!(allowed_tools.default.unwrap().contains("ARRAY"));
        let extra_headers = column(&client, "LiteLLM_MCPServerTable", "extra_headers").await;
        assert!(extra_headers.is_nullable);
        assert!(extra_headers.default.unwrap().contains("ARRAY"));
    })
    .await;
}

#[tokio::test]
async fn static_headers_credentials_empty_raw_json() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20251104220043_add_credentials_to_mcp_servers"),
        )
        .await;
        let static_headers = column(&client, "LiteLLM_MCPServerTable", "static_headers").await;
        assert!(static_headers.is_nullable);
        assert!(static_headers.default.unwrap().contains("{}"));
        let credentials = column(&client, "LiteLLM_MCPServerTable", "credentials").await;
        assert!(credentials.is_nullable);
        assert!(credentials.default.unwrap().contains("{}"));
    })
    .await;
}

#[tokio::test]
async fn nullable_tool_permissions() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20251006143948_add_mcp_tool_permissions"),
        )
        .await;
        let info = column(
            &client,
            "LiteLLM_ObjectPermissionTable",
            "mcp_tool_permissions",
        )
        .await;
        assert!(info.is_nullable);
        assert!(info.default.is_none());
    })
    .await;
}

async fn assert_empty_array_default(client: &Client, table: &str, name: &str) {
    let info = column(client, table, name).await;
    assert!(info.is_nullable, "{table}.{name} should stay nullable");
    assert!(
        info.default.unwrap().contains("ARRAY"),
        "{table}.{name} should default to an empty array"
    );
}

#[tokio::test]
async fn empty_agent_permissions() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_all()).await;
        assert_empty_array_default(&client, "LiteLLM_ObjectPermissionTable", "agents").await;
        assert_empty_array_default(
            &client,
            "LiteLLM_ObjectPermissionTable",
            "agent_access_groups",
        )
        .await;
        assert_empty_array_default(&client, "LiteLLM_AgentsTable", "agent_access_groups").await;
    })
    .await;
}
