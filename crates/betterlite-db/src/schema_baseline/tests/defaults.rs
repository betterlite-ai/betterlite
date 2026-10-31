#[tokio::test]
async fn baseline_defaults() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let created_at = column(&client, "LiteLLM_BudgetTable", "created_at").await;
        assert_eq!(created_at.default.unwrap(), "CURRENT_TIMESTAMP");
        let teams = column(&client, "LiteLLM_UserTable", "teams").await;
        assert!(teams.default.is_some());
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_budget_nullable_limits_and_required_actors() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let max_budget = column(&client, "LiteLLM_BudgetTable", "max_budget").await;
        assert_eq!(max_budget.data_type, "double precision");
        assert!(max_budget.is_nullable);
        assert_eq!(max_budget.default, None);
        let created_by = column(&client, "LiteLLM_BudgetTable", "created_by").await;
        assert_eq!(created_by.data_type, "text");
        assert!(!created_by.is_nullable);
        assert_eq!(created_by.default, None);
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_credentials_required_values_nullable_info() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let values = column(&client, "LiteLLM_CredentialsTable", "credential_values").await;
        assert_eq!(values.data_type, "jsonb");
        assert!(!values.is_nullable);
        let info = column(&client, "LiteLLM_CredentialsTable", "credential_info").await;
        assert_eq!(info.data_type, "jsonb");
        assert!(info.is_nullable);
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_proxy_model_required_params_nullable_info() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let params = column(&client, "LiteLLM_ProxyModelTable", "litellm_params").await;
        assert_eq!(params.data_type, "jsonb");
        assert!(!params.is_nullable);
        let info = column(&client, "LiteLLM_ProxyModelTable", "model_info").await;
        assert!(info.is_nullable);
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_organization_json_spend_and_nullable_models() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let models = column(&client, "LiteLLM_OrganizationTable", "models").await;
        assert_eq!(models.data_type, "ARRAY");
        assert!(models.is_nullable);
        assert_eq!(models.default, None);
        let spend = column(&client, "LiteLLM_OrganizationTable", "spend").await;
        assert!(!spend.is_nullable);
        assert!(is_zero(spend.default));
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_serial_model_id_and_nullable_aliases() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let id = column(&client, "LiteLLM_ModelTable", "id").await;
        assert_eq!(id.data_type, "integer");
        assert!(!id.is_nullable);
        assert!(id.default.unwrap().starts_with("nextval("));
        let aliases = column(&client, "LiteLLM_ModelTable", "aliases").await;
        assert_eq!(aliases.data_type, "jsonb");
        assert!(aliases.is_nullable);
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_team_object_roles_false_blocked_and_nullable_arrays() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let roles = column(&client, "LiteLLM_TeamTable", "members_with_roles").await;
        assert_eq!(roles.data_type, "jsonb");
        assert!(!roles.is_nullable);
        assert!(roles.default.unwrap().contains("{}"));
        let blocked = column(&client, "LiteLLM_TeamTable", "blocked").await;
        assert_eq!(blocked.data_type, "boolean");
        assert!(!blocked.is_nullable);
        assert!(blocked.default.unwrap().contains("false"));
        let models = column(&client, "LiteLLM_TeamTable", "models").await;
        assert_eq!(models.data_type, "ARRAY");
        assert!(models.is_nullable);
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_user_nullable_empty_arrays_and_timestamps() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let teams = column(&client, "LiteLLM_UserTable", "teams").await;
        assert_eq!(teams.data_type, "ARRAY");
        assert!(teams.is_nullable);
        assert!(teams.default.is_some());
        let created_at = column(&client, "LiteLLM_UserTable", "created_at").await;
        assert!(created_at.is_nullable);
        assert!(created_at.default.is_some());
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_key_six_objects_false_cooldown_and_nullable_blocked() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let cooldown = column(&client, "LiteLLM_VerificationToken", "soft_budget_cooldown").await;
        assert_eq!(cooldown.data_type, "boolean");
        assert!(!cooldown.is_nullable);
        assert!(cooldown.default.unwrap().contains("false"));
        let aliases = column(&client, "LiteLLM_VerificationToken", "aliases").await;
        assert_eq!(aliases.data_type, "jsonb");
        assert!(!aliases.is_nullable);
        assert!(aliases.default.unwrap().contains("{}"));
        let config = column(&client, "LiteLLM_VerificationToken", "config").await;
        assert_eq!(config.data_type, "jsonb");
        assert!(!config.is_nullable);
        assert!(config.default.unwrap().contains("{}"));
        let permissions = column(&client, "LiteLLM_VerificationToken", "permissions").await;
        assert_eq!(permissions.data_type, "jsonb");
        assert!(!permissions.is_nullable);
        assert!(permissions.default.unwrap().contains("{}"));
        let metadata = column(&client, "LiteLLM_VerificationToken", "metadata").await;
        assert_eq!(metadata.data_type, "jsonb");
        assert!(!metadata.is_nullable);
        assert!(metadata.default.unwrap().contains("{}"));
        let model_spend = column(&client, "LiteLLM_VerificationToken", "model_spend").await;
        assert_eq!(model_spend.data_type, "jsonb");
        assert!(!model_spend.is_nullable);
        assert!(model_spend.default.unwrap().contains("{}"));
        let model_max_budget =
            column(&client, "LiteLLM_VerificationToken", "model_max_budget").await;
        assert_eq!(model_max_budget.data_type, "jsonb");
        assert!(!model_max_budget.is_nullable);
        assert!(model_max_budget.default.unwrap().contains("{}"));
        let blocked = column(&client, "LiteLLM_VerificationToken", "blocked").await;
        assert!(blocked.is_nullable);
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_end_user_false_blocked() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let blocked = column(&client, "LiteLLM_EndUserTable", "blocked").await;
        assert_eq!(blocked.data_type, "boolean");
        assert!(!blocked.is_nullable);
        assert!(blocked.default.unwrap().contains("false"));
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_config_nullable_json() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let value = column(&client, "LiteLLM_Config", "param_value").await;
        assert_eq!(value.data_type, "jsonb");
        assert!(value.is_nullable);
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_spend_log_empty_strings_json_arrays_and_required_times() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let api_key = column(&client, "LiteLLM_SpendLogs", "api_key").await;
        assert!(!api_key.is_nullable);
        assert!(api_key.default.unwrap().contains("''"));
        let request_tags = column(&client, "LiteLLM_SpendLogs", "request_tags").await;
        assert_eq!(request_tags.data_type, "jsonb");
        assert!(request_tags.default.unwrap().contains("[]"));
        let start_time = column(&client, "LiteLLM_SpendLogs", "startTime").await;
        assert!(!start_time.is_nullable);
        assert_eq!(start_time.default, None);
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_error_log_text_status_and_required_times() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let status = column(&client, "LiteLLM_ErrorLogs", "status_code").await;
        assert_eq!(status.data_type, "text");
        assert!(!status.is_nullable);
        assert!(status.default.unwrap().contains("''"));
        let end_time = column(&client, "LiteLLM_ErrorLogs", "endTime").await;
        assert!(!end_time.is_nullable);
        assert_eq!(end_time.default, None);
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_notification_required_status_nullable_models() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let status = column(&client, "LiteLLM_UserNotifications", "status").await;
        assert!(!status.is_nullable);
        let models = column(&client, "LiteLLM_UserNotifications", "models").await;
        assert_eq!(models.data_type, "ARRAY");
        assert!(models.is_nullable);
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_team_membership_required_spend() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let spend = column(&client, "LiteLLM_TeamMembership", "spend").await;
        assert!(!spend.is_nullable);
        assert!(is_zero(spend.default));
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_organization_membership_nullable_defaults() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let spend = column(&client, "LiteLLM_OrganizationMembership", "spend").await;
        assert!(spend.is_nullable);
        assert!(is_zero(spend.default));
        let created_at = column(&client, "LiteLLM_OrganizationMembership", "created_at").await;
        assert!(created_at.is_nullable);
        assert!(created_at.default.is_some());
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_invitation_no_timestamp_defaults() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let expires_at = column(&client, "LiteLLM_InvitationLink", "expires_at").await;
        assert!(!expires_at.is_nullable);
        assert_eq!(expires_at.default, None);
        let created_at = column(&client, "LiteLLM_InvitationLink", "created_at").await;
        assert!(!created_at.is_nullable);
        assert_eq!(created_at.default, None);
    })
    .await;
}

#[tokio::test]
async fn baseline_defaults_audit_empty_actor_defaults() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let changed_by = column(&client, "LiteLLM_AuditLog", "changed_by").await;
        assert!(!changed_by.is_nullable);
        assert!(changed_by.default.unwrap().contains("''"));
        let changed_by_api_key = column(&client, "LiteLLM_AuditLog", "changed_by_api_key").await;
        assert!(!changed_by_api_key.is_nullable);
        assert!(changed_by_api_key.default.unwrap().contains("''"));
    })
    .await;
}
