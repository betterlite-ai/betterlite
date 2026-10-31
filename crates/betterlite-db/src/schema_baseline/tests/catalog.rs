#[rstest]
#[case::budget_table("LiteLLM_BudgetTable")]
#[case::credentials_table("LiteLLM_CredentialsTable")]
#[case::proxy_model_table("LiteLLM_ProxyModelTable")]
#[case::organization_table("LiteLLM_OrganizationTable")]
#[case::model_table("LiteLLM_ModelTable")]
#[case::team_table("LiteLLM_TeamTable")]
#[case::user_table("LiteLLM_UserTable")]
#[case::verification_token("LiteLLM_VerificationToken")]
#[case::end_user_table("LiteLLM_EndUserTable")]
#[case::config("LiteLLM_Config")]
#[case::spend_logs("LiteLLM_SpendLogs")]
#[case::error_logs("LiteLLM_ErrorLogs")]
#[case::user_notifications("LiteLLM_UserNotifications")]
#[case::team_membership("LiteLLM_TeamMembership")]
#[case::organization_membership("LiteLLM_OrganizationMembership")]
#[case::invitation_link("LiteLLM_InvitationLink")]
#[case::audit_log("LiteLLM_AuditLog")]
#[tokio::test]
async fn baseline_catalog_columns(#[case] table: &'static str) {
    Box::pin(harness2(
        move |client_candidate, client_upstream| async move {
            client_candidate
                .batch_execute(migration_sql())
                .await
                .unwrap();
            client_upstream
                .batch_execute(include_str!(
                    "../../../../../tools/postgres-fixture/fixtures/baseline.sql"
                ))
                .await
                .unwrap();
            let (candidate, candidate_pk) = table_shape(&client_candidate, "public", table).await;
            let (upstream, upstream_pk) = table_shape(&client_upstream, "public", table).await;
            assert_eq!(candidate, upstream, "{table} columns differ from upstream");
            assert!(!candidate.is_empty(), "{table} has no columns");
            assert_eq!(
                candidate_pk, upstream_pk,
                "{table} primary key differs from upstream"
            );
            assert!(!candidate_pk.is_empty(), "{table} has no primary key");
        },
    ))
    .await;
}
