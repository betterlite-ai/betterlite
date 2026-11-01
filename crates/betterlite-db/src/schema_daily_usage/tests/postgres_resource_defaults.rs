const NEW_CRON_JOB_TABLE: &str = "20250329084805_new_cron_job_table";
const ADD_ALLOWED_ROUTES: &str = "20250416185146_add_allowed_routes_litellm_verification_token";
const ADD_SESSION_ID_INDEX: &str = "20250510142544_add_session_id_index_spend_logs";
const ADD_MANAGED_FILE_TABLE: &str = "20250411215431_add_managed_file_table";
const MANAGED_OBJECT_TABLE: &str = "20250522223020_managed_object_table";
const ADD_MANAGED_VECTOR_STORES: &str = "20250430193429_add_managed_vector_stores";
const ADD_MCP_TABLE_TO_DB: &str = "20250507161526_add_mcp_table_to_db";
const ADD_HEALTH_CHECK_FIELDS_TO_MCP_SERVERS: &str =
    "20250507161527_add_health_check_fields_to_mcp_servers";
const ADD_VECTOR_STORES_TO_OBJECT_PERMISSIONS: &str =
    "20250528185438_add_vector_stores_to_object_permissions";
const ADD_GUARDRAILS_TABLE: &str = "20250514142245_add_guardrails_table";
const ADD_HEALTH_CHECK_TABLE: &str = "20250618225828_add_health_check_table";
const ADD_STATUS_TO_MANAGED_OBJECT_TABLE: &str =
    "20250625213625_add_status_to_managed_object_table";
const ADD_MCP_INFO_COLUMN_MCP_SERVERS: &str = "20250707212517_add_mcp_info_column_mcp_servers";

#[tokio::test]
async fn cron_inactive_and_required_ttl() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(NEW_CRON_JOB_TABLE)).await;
        exec_params(
            &client,
            r#"INSERT INTO "LiteLLM_CronJob" (cronjob_id, pod_id, ttl) VALUES ('c1', 'p1', NOW())"#,
            &[],
        )
        .await;
        let status = scalar_text(
            &client,
            r#"SELECT status::text FROM "LiteLLM_CronJob" WHERE cronjob_id = 'c1'"#,
        )
        .await;
        assert_eq!(status, "INACTIVE");
        let ttl = column(&client, "LiteLLM_CronJob", "ttl").await;
        assert!(!ttl.is_nullable);
        assert_eq!(ttl.default, None);
    })
    .await;
}

#[tokio::test]
async fn cron_enum_repeat_42710() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(NEW_CRON_JOB_TABLE)).await;
        let error = batch_execute_err(&client, migration_sql_of(NEW_CRON_JOB_TABLE)).await;
        assert_eq!(error.code().unwrap().code(), "42710");
    })
    .await;
}

#[tokio::test]
async fn team_permissions_and_key_routes_empty_nullable_arrays() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(ADD_ALLOWED_ROUTES)).await;
        let permissions = column(&client, "LiteLLM_TeamTable", "team_member_permissions").await;
        assert_eq!(permissions.data_type, "ARRAY");
        assert!(permissions.is_nullable);
        assert!(permissions.default.is_some());
        let routes = column(&client, "LiteLLM_VerificationToken", "allowed_routes").await;
        assert_eq!(routes.data_type, "ARRAY");
        assert!(routes.is_nullable);
        assert!(routes.default.is_some());
    })
    .await;
}

#[tokio::test]
async fn spend_proxy_json_session_status_and_index() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(ADD_SESSION_ID_INDEX)).await;
        let proxy_request = column(&client, "LiteLLM_SpendLogs", "proxy_server_request").await;
        assert_eq!(proxy_request.data_type, "jsonb");
        assert!(proxy_request.is_nullable);
        assert!(proxy_request.default.unwrap().contains("{}"));
        let session_id = column(&client, "LiteLLM_SpendLogs", "session_id").await;
        assert!(session_id.is_nullable);
        let status = column(&client, "LiteLLM_SpendLogs", "status").await;
        assert!(status.is_nullable);
        assert!(index_exists(&client, "LiteLLM_SpendLogs_session_id_idx").await);
    })
    .await;
}

#[tokio::test]
async fn managed_file_initial_required_json_then_nullable() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(ADD_MANAGED_FILE_TABLE)).await;
        let initial = column(&client, "LiteLLM_ManagedFileTable", "file_object").await;
        assert!(!initial.is_nullable);
        batch_execute(
            &client,
            &migration_sql_between(
                ADD_MANAGED_FILE_TABLE,
                CASCADE_BUDGET_AND_LOOSEN_MANAGED_FILE_JSON,
            ),
        )
        .await;
        let later = column(&client, "LiteLLM_ManagedFileTable", "file_object").await;
        assert!(later.is_nullable);
    })
    .await;
}

#[tokio::test]
async fn managed_file_actor_and_flat_ids() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(MANAGED_OBJECT_TABLE)).await;
        let created_by = column(&client, "LiteLLM_ManagedFileTable", "created_by").await;
        assert!(created_by.is_nullable);
        let updated_by = column(&client, "LiteLLM_ManagedFileTable", "updated_by").await;
        assert!(updated_by.is_nullable);
        let flat_ids = column(&client, "LiteLLM_ManagedFileTable", "flat_model_file_ids").await;
        assert_eq!(flat_ids.data_type, "ARRAY");
        assert!(flat_ids.is_nullable);
        assert!(flat_ids.default.is_some());
    })
    .await;
}

async fn assert_nullable_columns(client: &Client, table: &str, names: &[&str]) {
    for name in names {
        let info = column(client, table, name).await;
        assert!(info.is_nullable, "{name}");
    }
}

#[tokio::test]
async fn plural_vector_store_fields() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(ADD_MANAGED_VECTOR_STORES)).await;
        let id = column(
            &client,
            "LiteLLM_ManagedVectorStoresTable",
            "vector_store_id",
        )
        .await;
        assert!(!id.is_nullable);
        let provider = column(
            &client,
            "LiteLLM_ManagedVectorStoresTable",
            "custom_llm_provider",
        )
        .await;
        assert!(!provider.is_nullable);
        assert_nullable_columns(
            &client,
            "LiteLLM_ManagedVectorStoresTable",
            &[
                "vector_store_name",
                "vector_store_description",
                "vector_store_metadata",
                "litellm_credential_name",
            ],
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn mcp_sse_protocol_and_unknown_health() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(ADD_MCP_TABLE_TO_DB)).await;
        let transport = column(&client, "LiteLLM_MCPServerTable", "transport").await;
        assert!(transport.default.unwrap().contains("sse"));
        let spec_version = column(&client, "LiteLLM_MCPServerTable", "spec_version").await;
        assert!(spec_version.default.unwrap().contains("2025-03-26"));
        batch_execute(
            &client,
            &migration_sql_between(ADD_MCP_TABLE_TO_DB, ADD_HEALTH_CHECK_FIELDS_TO_MCP_SERVERS),
        )
        .await;
        let status = column(&client, "LiteLLM_MCPServerTable", "status").await;
        assert!(status.is_nullable);
        assert!(status.default.unwrap().contains("unknown"));
    })
    .await;
}

#[tokio::test]
async fn mcp_info_empty_object() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through(ADD_MCP_INFO_COLUMN_MCP_SERVERS),
        )
        .await;
        let info = column(&client, "LiteLLM_MCPServerTable", "mcp_info").await;
        assert_eq!(info.data_type, "jsonb");
        assert!(info.is_nullable);
        assert!(info.default.unwrap().contains("{}"));
    })
    .await;
}

async fn assert_restrict_rules(client: &Client, conames: &[&str]) {
    for conname in conames {
        assert_rule(client, conname, 'n').await;
    }
}

#[tokio::test]
async fn object_permission_empty_arrays_and_four_set_null_foreign_keys() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through(ADD_VECTOR_STORES_TO_OBJECT_PERMISSIONS),
        )
        .await;
        let mcp_servers = column(&client, "LiteLLM_ObjectPermissionTable", "mcp_servers").await;
        assert!(mcp_servers.is_nullable);
        assert!(mcp_servers.default.is_some());
        let vector_stores = column(&client, "LiteLLM_ObjectPermissionTable", "vector_stores").await;
        assert!(vector_stores.is_nullable);
        assert!(vector_stores.default.is_some());
        assert_restrict_rules(
            &client,
            &[
                "LiteLLM_OrganizationTable_object_permission_id_fkey",
                "LiteLLM_TeamTable_object_permission_id_fkey",
                "LiteLLM_UserTable_object_permission_id_fkey",
                "LiteLLM_VerificationToken_object_permission_id_fkey",
            ],
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn guardrail_unique_name_and_required_params() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(ADD_GUARDRAILS_TABLE)).await;
        let facts = index_facts(&client, "LiteLLM_GuardrailsTable_guardrail_name_key").await;
        assert!(facts.unique);
        assert_eq!(facts.columns, ["guardrail_name"]);
        let params = column(&client, "LiteLLM_GuardrailsTable", "litellm_params").await;
        assert!(!params.is_nullable);
        let updated_at = column(&client, "LiteLLM_GuardrailsTable", "updated_at").await;
        assert!(!updated_at.is_nullable);
        assert_eq!(updated_at.default, None);
    })
    .await;
}

#[tokio::test]
async fn managed_object_two_unique_and_two_lookup_indexes() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(MANAGED_OBJECT_TABLE)).await;
        let unique_unified =
            index_facts(&client, "LiteLLM_ManagedObjectTable_unified_object_id_key").await;
        assert!(unique_unified.unique);
        let lookup_unified =
            index_facts(&client, "LiteLLM_ManagedObjectTable_unified_object_id_idx").await;
        assert!(!lookup_unified.unique);
        let unique_model =
            index_facts(&client, "LiteLLM_ManagedObjectTable_model_object_id_key").await;
        assert!(unique_model.unique);
        let lookup_model =
            index_facts(&client, "LiteLLM_ManagedObjectTable_model_object_id_idx").await;
        assert!(!lookup_model.unique);
    })
    .await;
}

#[tokio::test]
async fn managed_object_nullable_status() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through(ADD_STATUS_TO_MANAGED_OBJECT_TABLE),
        )
        .await;
        let status = column(&client, "LiteLLM_ManagedObjectTable", "status").await;
        assert!(status.is_nullable);
    })
    .await;
}

async fn assert_indexes_exist(client: &Client, indexes: &[&str]) {
    for index in indexes {
        assert!(index_exists(client, index).await, "{index}");
    }
}

#[tokio::test]
async fn health_check_defaults_and_three_indexes() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(ADD_HEALTH_CHECK_TABLE)).await;
        let healthy = column(&client, "LiteLLM_HealthCheckTable", "healthy_count").await;
        assert!(!healthy.is_nullable);
        assert!(healthy.default.unwrap().contains('0'));
        let unhealthy = column(&client, "LiteLLM_HealthCheckTable", "unhealthy_count").await;
        assert!(!unhealthy.is_nullable);
        assert!(unhealthy.default.unwrap().contains('0'));
        let updated_at = column(&client, "LiteLLM_HealthCheckTable", "updated_at").await;
        assert!(!updated_at.is_nullable);
        assert_eq!(updated_at.default, None);
        assert_indexes_exist(
            &client,
            &[
                "LiteLLM_HealthCheckTable_model_name_idx",
                "LiteLLM_HealthCheckTable_checked_at_idx",
                "LiteLLM_HealthCheckTable_status_idx",
            ],
        )
        .await;
    })
    .await;
}
