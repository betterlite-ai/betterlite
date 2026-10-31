#[tokio::test]
async fn baseline_constraints_credential_name_unique() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        exec(
            &client,
            r#"INSERT INTO "LiteLLM_CredentialsTable" (credential_id, credential_name, credential_values, created_by, updated_by) VALUES ('c1', 'dup', '{}', 't', 't')"#,
        )
        .await;
        let error = exec_err(
            &client,
            r#"INSERT INTO "LiteLLM_CredentialsTable" (credential_id, credential_name, credential_values, created_by, updated_by) VALUES ('c2', 'dup', '{}', 't', 't')"#,
        )
        .await;
        assert_eq!(error.code(), Some(&SqlState::UNIQUE_VIOLATION));
        let facts = index_facts(&client, "LiteLLM_CredentialsTable_credential_name_key").await;
        assert_eq!(facts.columns, vec!["credential_name"]);
        assert!(facts.unique);
    })
    .await;
}

#[tokio::test]
async fn baseline_constraints_team_model_unique_nulls() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        exec(
            &client,
            r#"INSERT INTO "LiteLLM_TeamTable" (team_id) VALUES ('t1')"#,
        )
        .await;
        exec(
            &client,
            r#"INSERT INTO "LiteLLM_TeamTable" (team_id) VALUES ('t2')"#,
        )
        .await;
        exec(
            &client,
            r#"INSERT INTO "LiteLLM_ModelTable" (id, created_by, updated_by) VALUES (9001, 't', 't')"#,
        )
        .await;
        exec(
            &client,
            r#"UPDATE "LiteLLM_TeamTable" SET model_id = 9001 WHERE team_id = 't1'"#,
        )
        .await;
        let error = exec_err(
            &client,
            r#"UPDATE "LiteLLM_TeamTable" SET model_id = 9001 WHERE team_id = 't2'"#,
        )
        .await;
        assert_eq!(error.code(), Some(&SqlState::UNIQUE_VIOLATION));
        let facts = index_facts(&client, "LiteLLM_TeamTable_model_id_key").await;
        assert_eq!(facts.columns, vec!["model_id"]);
        assert!(facts.unique);
    })
    .await;
}

#[tokio::test]
async fn baseline_constraints_sso_id_unique_nulls() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        exec(
            &client,
            r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u1')"#,
        )
        .await;
        exec(
            &client,
            r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u2')"#,
        )
        .await;
        exec(
            &client,
            r#"UPDATE "LiteLLM_UserTable" SET sso_user_id = 'sso1' WHERE user_id = 'u1'"#,
        )
        .await;
        let error = exec_err(
            &client,
            r#"UPDATE "LiteLLM_UserTable" SET sso_user_id = 'sso1' WHERE user_id = 'u2'"#,
        )
        .await;
        assert_eq!(error.code(), Some(&SqlState::UNIQUE_VIOLATION));
        let facts = index_facts(&client, "LiteLLM_UserTable_sso_user_id_key").await;
        assert_eq!(facts.columns, vec!["sso_user_id"]);
        assert!(facts.unique);
    })
    .await;
}

#[tokio::test]
async fn baseline_constraints_spend_time_and_end_user_indexes() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let start_time = index_facts(&client, "LiteLLM_SpendLogs_startTime_idx").await;
        assert_eq!(start_time.columns, vec!["startTime"]);
        assert!(!start_time.unique);
        let end_user = index_facts(&client, "LiteLLM_SpendLogs_end_user_idx").await;
        assert_eq!(end_user.columns, vec!["end_user"]);
        assert!(!end_user.unique);
    })
    .await;
}

#[tokio::test]
async fn baseline_constraints_redundant_membership_unique_index() {
    harness(|client| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        let redundant = index_facts(
            &client,
            "LiteLLM_OrganizationMembership_user_id_organization_id_key",
        )
        .await;
        assert_eq!(redundant.columns, vec!["user_id", "organization_id"]);
        assert!(redundant.unique);
        let primary_key = index_facts(&client, "LiteLLM_OrganizationMembership_pkey").await;
        assert_eq!(primary_key.columns, vec!["user_id", "organization_id"]);
        assert!(primary_key.unique);
    })
    .await;
}
