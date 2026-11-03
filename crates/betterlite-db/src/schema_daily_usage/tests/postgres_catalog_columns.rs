type CatalogColumn = (
    &'static str,
    &'static str,
    bool,
    Option<&'static str>,
    &'static str,
    Option<i32>,
);

#[rstest]
#[case::daily_user_spend("LiteLLM_DailyUserSpend", &[("id", "text", false, None, "text", None), ("user_id", "text", true, None, "text", None), ("date", "text", false, None, "text", None), ("api_key", "text", false, None, "text", None), ("model", "text", false, None, "text", None), ("model_group", "text", true, None, "text", None), ("custom_llm_provider", "text", true, None, "text", None), ("prompt_tokens", "bigint", false, Some("0"), "int8", None), ("completion_tokens", "bigint", false, Some("0"), "int8", None), ("spend", "double precision", false, Some("0.0"), "float8", None), ("created_at", "timestamp without time zone", false, Some("CURRENT_TIMESTAMP"), "timestamp", Some(3)), ("updated_at", "timestamp without time zone", false, None, "timestamp", Some(3)), ("api_requests", "bigint", false, Some("0"), "int8", None), ("failed_requests", "bigint", false, Some("0"), "int8", None), ("successful_requests", "bigint", false, Some("0"), "int8", None), ("cache_creation_input_tokens", "bigint", false, Some("0"), "int8", None), ("cache_read_input_tokens", "bigint", false, Some("0"), "int8", None)])]
#[case::cron_job("LiteLLM_CronJob", &[("cronjob_id", "text", false, None, "text", None), ("pod_id", "text", false, None, "text", None), ("status", "USER-DEFINED", false, Some("'INACTIVE'::\"JobStatus\""), "JobStatus", None), ("last_updated", "timestamp without time zone", false, Some("CURRENT_TIMESTAMP"), "timestamp", Some(3)), ("ttl", "timestamp without time zone", false, None, "timestamp", Some(3))])]
#[case::managed_file_table("LiteLLM_ManagedFileTable", &[("id", "text", false, None, "text", None), ("unified_file_id", "text", false, None, "text", None), ("file_object", "jsonb", true, None, "jsonb", None), ("model_mappings", "jsonb", false, None, "jsonb", None), ("created_at", "timestamp without time zone", false, Some("CURRENT_TIMESTAMP"), "timestamp", Some(3)), ("updated_at", "timestamp without time zone", false, None, "timestamp", Some(3)), ("created_by", "text", true, None, "text", None), ("flat_model_file_ids", "ARRAY", true, Some("ARRAY[]::text[]"), "_text", None), ("updated_by", "text", true, None, "text", None)])]
#[case::daily_team_spend("LiteLLM_DailyTeamSpend", &[("id", "text", false, None, "text", None), ("team_id", "text", true, None, "text", None), ("date", "text", false, None, "text", None), ("api_key", "text", false, None, "text", None), ("model", "text", false, None, "text", None), ("model_group", "text", true, None, "text", None), ("custom_llm_provider", "text", true, None, "text", None), ("prompt_tokens", "bigint", false, Some("0"), "int8", None), ("completion_tokens", "bigint", false, Some("0"), "int8", None), ("spend", "double precision", false, Some("0.0"), "float8", None), ("api_requests", "bigint", false, Some("0"), "int8", None), ("successful_requests", "bigint", false, Some("0"), "int8", None), ("failed_requests", "bigint", false, Some("0"), "int8", None), ("created_at", "timestamp without time zone", false, Some("CURRENT_TIMESTAMP"), "timestamp", Some(3)), ("updated_at", "timestamp without time zone", false, None, "timestamp", Some(3)), ("cache_creation_input_tokens", "bigint", false, Some("0"), "int8", None), ("cache_read_input_tokens", "bigint", false, Some("0"), "int8", None)])]
#[case::daily_tag_spend("LiteLLM_DailyTagSpend", &[("id", "text", false, None, "text", None), ("tag", "text", true, None, "text", None), ("date", "text", false, None, "text", None), ("api_key", "text", false, None, "text", None), ("model", "text", false, None, "text", None), ("model_group", "text", true, None, "text", None), ("custom_llm_provider", "text", true, None, "text", None), ("prompt_tokens", "bigint", false, Some("0"), "int8", None), ("completion_tokens", "bigint", false, Some("0"), "int8", None), ("cache_read_input_tokens", "bigint", false, Some("0"), "int8", None), ("cache_creation_input_tokens", "bigint", false, Some("0"), "int8", None), ("spend", "double precision", false, Some("0.0"), "float8", None), ("api_requests", "bigint", false, Some("0"), "int8", None), ("successful_requests", "bigint", false, Some("0"), "int8", None), ("failed_requests", "bigint", false, Some("0"), "int8", None), ("created_at", "timestamp without time zone", false, Some("CURRENT_TIMESTAMP"), "timestamp", Some(3)), ("updated_at", "timestamp without time zone", false, None, "timestamp", Some(3))])]
#[case::managed_vector_stores_table("LiteLLM_ManagedVectorStoresTable", &[("vector_store_id", "text", false, None, "text", None), ("custom_llm_provider", "text", false, None, "text", None), ("vector_store_name", "text", true, None, "text", None), ("vector_store_description", "text", true, None, "text", None), ("vector_store_metadata", "jsonb", true, None, "jsonb", None), ("created_at", "timestamp without time zone", false, Some("CURRENT_TIMESTAMP"), "timestamp", Some(3)), ("updated_at", "timestamp without time zone", false, None, "timestamp", Some(3)), ("litellm_credential_name", "text", true, None, "text", None)])]
#[case::mcp_server_table("LiteLLM_MCPServerTable", &[("server_id", "text", false, None, "text", None), ("alias", "text", true, None, "text", None), ("description", "text", true, None, "text", None), ("url", "text", false, None, "text", None), ("transport", "text", false, Some("'sse'::text"), "text", None), ("spec_version", "text", false, Some("'2025-03-26'::text"), "text", None), ("auth_type", "text", true, None, "text", None), ("created_at", "timestamp without time zone", true, Some("CURRENT_TIMESTAMP"), "timestamp", Some(3)), ("created_by", "text", true, None, "text", None), ("updated_at", "timestamp without time zone", true, Some("CURRENT_TIMESTAMP"), "timestamp", Some(3)), ("updated_by", "text", true, None, "text", None), ("status", "text", true, Some("'unknown'::text"), "text", None), ("last_health_check", "timestamp without time zone", true, None, "timestamp", Some(3)), ("health_check_error", "text", true, None, "text", None), ("mcp_info", "jsonb", true, Some("'{}'::jsonb"), "jsonb", None)])]
#[case::object_permission_table("LiteLLM_ObjectPermissionTable", &[("object_permission_id", "text", false, None, "text", None), ("mcp_servers", "ARRAY", true, Some("ARRAY[]::text[]"), "_text", None), ("vector_stores", "ARRAY", true, Some("ARRAY[]::text[]"), "_text", None)])]
#[case::guardrails_table("LiteLLM_GuardrailsTable", &[("guardrail_id", "text", false, None, "text", None), ("guardrail_name", "text", false, None, "text", None), ("litellm_params", "jsonb", false, None, "jsonb", None), ("guardrail_info", "jsonb", true, None, "jsonb", None), ("created_at", "timestamp without time zone", false, Some("CURRENT_TIMESTAMP"), "timestamp", Some(3)), ("updated_at", "timestamp without time zone", false, None, "timestamp", Some(3))])]
#[case::managed_object_table("LiteLLM_ManagedObjectTable", &[("id", "text", false, None, "text", None), ("unified_object_id", "text", false, None, "text", None), ("model_object_id", "text", false, None, "text", None), ("file_object", "jsonb", false, None, "jsonb", None), ("file_purpose", "text", false, None, "text", None), ("created_at", "timestamp without time zone", false, Some("CURRENT_TIMESTAMP"), "timestamp", Some(3)), ("created_by", "text", true, None, "text", None), ("updated_at", "timestamp without time zone", false, None, "timestamp", Some(3)), ("updated_by", "text", true, None, "text", None), ("status", "text", true, None, "text", None)])]
#[case::health_check_table("LiteLLM_HealthCheckTable", &[("health_check_id", "text", false, None, "text", None), ("model_name", "text", false, None, "text", None), ("model_id", "text", true, None, "text", None), ("status", "text", false, None, "text", None), ("healthy_count", "integer", false, Some("0"), "int4", None), ("unhealthy_count", "integer", false, Some("0"), "int4", None), ("error_message", "text", true, None, "text", None), ("response_time_ms", "double precision", true, None, "float8", None), ("details", "jsonb", true, None, "jsonb", None), ("checked_by", "text", true, None, "text", None), ("checked_at", "timestamp without time zone", false, Some("CURRENT_TIMESTAMP"), "timestamp", Some(3)), ("created_at", "timestamp without time zone", false, Some("CURRENT_TIMESTAMP"), "timestamp", Some(3)), ("updated_at", "timestamp without time zone", false, None, "timestamp", Some(3))])]
#[tokio::test]
async fn postgres_catalog_columns(
    #[case] table: &'static str,
    #[case] expected: &'static [CatalogColumn],
) {
    harness(move |[client]| async move {
        client.batch_execute(&migration_sql_all()).await.unwrap();
        assert_full_catalog_columns(&client, table, expected).await;
    })
    .await;
}

async fn assert_full_catalog_columns(client: &Client, table: &str, expected: &[CatalogColumn]) {
    let rows = columns_in(client, table).await;
    let names: Vec<String> = rows.iter().map(|row| row.get(1)).collect();
    let expected_names: Vec<&str> = expected.iter().map(|column| column.0).collect();
    assert_eq!(names, expected_names, "{table} columns in ordinal order");
    for (row, spec) in rows.iter().zip(expected) {
        let (_, data_type, nullable, default, udt_name, datetime_precision) = spec;
        assert_eq!(
            row.get::<_, String>(2),
            *data_type,
            "{table}.{} type",
            spec.0
        );
        assert_eq!(
            row.get::<_, String>(3) == "YES",
            *nullable,
            "{table}.{} nullable",
            spec.0
        );
        assert_eq!(
            row.get::<_, Option<String>>(4).as_deref(),
            *default,
            "{table}.{} default",
            spec.0
        );
        assert_eq!(
            row.get::<_, String>(5),
            *udt_name,
            "{table}.{} udt_name",
            spec.0
        );
        assert_eq!(
            row.get::<_, Option<i32>>(6),
            *datetime_precision,
            "{table}.{} precision",
            spec.0
        );
    }
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
        let rows = columns_in(&client, table).await;
        let actual: Vec<&str> = rows
            .iter()
            .filter_map(|row| {
                let name: String = row.get(1);
                expected.iter().find(|expected| **expected == name).copied()
            })
            .collect();
        assert_eq!(actual, expected, "{table} added columns in ordinal order");
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
