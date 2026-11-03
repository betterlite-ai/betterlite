#[rstest]
#[case::daily_user_spend("LiteLLM_DailyUserSpend", &["id", "user_id", "date", "api_key", "model", "model_group", "custom_llm_provider", "prompt_tokens", "completion_tokens", "spend", "created_at", "updated_at", "api_requests", "failed_requests", "successful_requests", "cache_creation_input_tokens", "cache_read_input_tokens"])]
#[case::cron_job("LiteLLM_CronJob", &["cronjob_id", "pod_id", "status", "last_updated", "ttl"])]
#[case::managed_file_table("LiteLLM_ManagedFileTable", &["id", "unified_file_id", "file_object", "model_mappings", "created_at", "updated_at", "created_by", "flat_model_file_ids", "updated_by"])]
#[case::daily_team_spend("LiteLLM_DailyTeamSpend", &["id", "team_id", "date", "api_key", "model", "model_group", "custom_llm_provider", "prompt_tokens", "completion_tokens", "spend", "api_requests", "successful_requests", "failed_requests", "created_at", "updated_at", "cache_creation_input_tokens", "cache_read_input_tokens"])]
#[case::daily_tag_spend("LiteLLM_DailyTagSpend", &["id", "tag", "date", "api_key", "model", "model_group", "custom_llm_provider", "prompt_tokens", "completion_tokens", "cache_read_input_tokens", "cache_creation_input_tokens", "spend", "api_requests", "successful_requests", "failed_requests", "created_at", "updated_at"])]
#[case::managed_vector_stores_table("LiteLLM_ManagedVectorStoresTable", &["vector_store_id", "custom_llm_provider", "vector_store_name", "vector_store_description", "vector_store_metadata", "created_at", "updated_at", "litellm_credential_name"])]
#[case::mcp_server_table("LiteLLM_MCPServerTable", &["server_id", "alias", "description", "url", "transport", "spec_version", "auth_type", "created_at", "created_by", "updated_at", "updated_by", "status", "last_health_check", "health_check_error", "mcp_info"])]
#[case::object_permission_table("LiteLLM_ObjectPermissionTable", &["object_permission_id", "mcp_servers", "vector_stores"])]
#[case::guardrails_table("LiteLLM_GuardrailsTable", &["guardrail_id", "guardrail_name", "litellm_params", "guardrail_info", "created_at", "updated_at"])]
#[case::managed_object_table("LiteLLM_ManagedObjectTable", &["id", "unified_object_id", "model_object_id", "file_object", "file_purpose", "created_at", "created_by", "updated_at", "updated_by", "status"])]
#[case::health_check_table("LiteLLM_HealthCheckTable", &["health_check_id", "model_name", "model_id", "status", "healthy_count", "unhealthy_count", "error_message", "response_time_ms", "details", "checked_by", "checked_at", "created_at", "updated_at"])]
#[tokio::test]
async fn postgres_catalog_columns(
    #[case] table: &'static str,
    #[case] expected: &'static [&'static str],
) {
    harness(move |[client]| async move {
        client.batch_execute(&migration_sql_all()).await.unwrap();
        let actual: Vec<String> = columns_in(&client, table)
            .await
            .iter()
            .map(|row| row.get(1))
            .collect();
        assert_eq!(actual, expected, "{table} columns in ordinal order");
    })
    .await;
}

#[rstest]
#[case::team_table("LiteLLM_TeamTable", &["team_member_permissions", "object_permission_id"])]
#[case::verification_token("LiteLLM_VerificationToken", &["allowed_routes", "object_permission_id"])]
#[case::user_table("LiteLLM_UserTable", &["object_permission_id"])]
#[case::organization_table("LiteLLM_OrganizationTable", &["object_permission_id"])]
#[case::spend_logs("LiteLLM_SpendLogs", &["proxy_server_request", "session_id", "status"])]
#[tokio::test]
async fn postgres_catalog_added_columns(
    #[case] table: &'static str,
    #[case] expected: &'static [&'static str],
) {
    harness(move |[client]| async move {
        client.batch_execute(&migration_sql_all()).await.unwrap();
        let actual: Vec<String> = columns_in(&client, table)
            .await
            .iter()
            .map(|row| row.get(1))
            .collect();
        let added: Vec<&str> = actual
            .iter()
            .filter_map(|column| {
                expected
                    .iter()
                    .find(|expected| **expected == column)
                    .copied()
            })
            .collect();
        assert_eq!(added, expected, "{table} added columns in ordinal order");
        assert_added_column_facts(&client, table, expected).await;
    })
    .await;
}

async fn assert_added_column_facts(client: &Client, table: &str, expected: &[&str]) {
    for name in expected {
        let info = column(client, table, name).await;
        assert!(info.is_nullable, "{table}.{name} nullable");
        match *name {
            "team_member_permissions" | "allowed_routes" => {
                assert_eq!(info.data_type, "ARRAY", "{table}.{name} type");
                assert_eq!(info.udt_name, "_text", "{table}.{name} udt_name");
                assert_eq!(
                    info.default.as_deref(),
                    Some("ARRAY[]::text[]"),
                    "{table}.{name} default"
                );
                assert_eq!(info.datetime_precision, None, "{table}.{name} precision");
            }
            "proxy_server_request" => {
                assert_eq!(info.data_type, "jsonb", "{table}.{name} type");
                assert_eq!(info.udt_name, "jsonb", "{table}.{name} udt_name");
                assert_eq!(
                    info.default.as_deref(),
                    Some("'{}'::jsonb"),
                    "{table}.{name} default"
                );
            }
            _ => {
                assert_eq!(info.data_type, "text", "{table}.{name} type");
                assert_eq!(info.udt_name, "text", "{table}.{name} udt_name");
                assert_eq!(info.default, None, "{table}.{name} default");
            }
        }
    }
}
