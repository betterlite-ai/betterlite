async fn check_agent_delete_live_cascade_retired_set_null(client: &Client) {
    insert_agent(client, "agent-i").await;
    insert_identity(client, "agent-i", "slack", "tuple-i").await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_RetiredAgentIdentity" (binding_id, agent_id, provider, issuer, tenant_id, client_id) VALUES ($1, $2, 'slack-retired', 'tuple-i-r', 'tuple-i-r', 'tuple-i-r')"#,
        &[&"binding-i", &"agent-i"],
    )
    .await;
    exec_params(
        client,
        r#"DELETE FROM "LiteLLM_AgentsTable" WHERE agent_id = $1"#,
        &[&"agent-i"],
    )
    .await;
    let identity = query_opt_row(
        client,
        r#"SELECT 1 FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
        &[&"agent-i"],
    )
    .await;
    assert!(identity.is_none());
    let retired_agent_id = scalar_opt_text(
        client,
        r#"SELECT agent_id FROM "LiteLLM_RetiredAgentIdentity" WHERE binding_id = $1"#,
        &[&"binding-i"],
    )
    .await;
    assert_eq!(retired_agent_id, None);
}
async fn check_user_delete_subject_cascade(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ($1)"#,
        &[&"user-j"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerifiedSubject" (subject_id, issuer, tenant_id, oid, user_id) VALUES ('subject-j', 'issuer-j', 'tenant-j', 'oid-j', $1)"#,
        &[&"user-j"],
    )
    .await;
    exec_params(
        client,
        r#"DELETE FROM "LiteLLM_UserTable" WHERE user_id = $1"#,
        &[&"user-j"],
    )
    .await;
    let subject = query_opt_row(
        client,
        r#"SELECT 1 FROM "LiteLLM_VerifiedSubject" WHERE subject_id = $1"#,
        &[&"subject-j"],
    )
    .await;
    assert!(subject.is_none());
}
async fn check_identity_parent_updates_cascade(client: &Client) {
    assert_rule(client, "LiteLLM_AgentIdentity_agent_id_fkey", 'c', 'c').await;
    assert_rule(
        client,
        "LiteLLM_RetiredAgentIdentity_agent_id_fkey",
        'c',
        'n',
    )
    .await;
    assert_rule(client, "LiteLLM_VerifiedSubject_user_id_fkey", 'c', 'c').await;
    insert_agent(client, "agent-k").await;
    insert_identity(client, "agent-k", "google", "tuple-k").await;
    exec_params(
        client,
        r#"UPDATE "LiteLLM_AgentsTable" SET agent_id = $2 WHERE agent_id = $1"#,
        &[&"agent-k", &"agent-k-renamed"],
    )
    .await;
    let old = query_opt_row(
        client,
        r#"SELECT 1 FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
        &[&"agent-k"],
    )
    .await;
    assert!(old.is_none());
    let renamed = scalar_text(
        client,
        r#"SELECT agent_id FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
        &[&"agent-k-renamed"],
    )
    .await;
    assert_eq!(renamed, "agent-k-renamed");
}
async fn check_no_automatic_retired_record(client: &Client) {
    insert_agent(client, "agent-l").await;
    exec_params(
        client,
        r#"DELETE FROM "LiteLLM_AgentsTable" WHERE agent_id = $1"#,
        &[&"agent-l"],
    )
    .await;
    let retired = query_opt_row(
        client,
        r#"SELECT 1 FROM "LiteLLM_RetiredAgent" WHERE original_agent_id = $1"#,
        &[&"agent-l"],
    )
    .await;
    assert!(retired.is_none());
}
fn identity_defaults_checkpoint(cohort: IdentityDefaultsCohort) -> &'static str {
    if matches!(
        cohort,
        IdentityDefaultsCohort::BillingAgentAndKillSwitchNullable
    ) {
        ADD_AGENT_KILL_SWITCH.directory
    } else {
        AGENT_IDENTITY.directory
    }
}
fn identity_defaults_check(
    client: &Client,
    cohort: IdentityDefaultsCohort,
) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
    use IdentityDefaultsCohort::{
        AgentDeleteLiveCascadeRetiredSetNull, AgentEnabledAutonomousUnmanaged,
        BillingAgentAndKillSwitchNullable, IdentityActiveAndNullableRoleScopeDefaults,
        IdentityParentUpdatesCascade, IdentityProviderTupleUnique, IssuerPrincipalUniqueWithNulls,
        LiveAndRetiredTupleUniquenessSeparate, NoAutomaticRetiredRecord, RetiredTimestampNoAgentFk,
        TextDefaultsNotEnumChecks, UserDeleteSubjectCascade,
        VerifiedHumanInteractiveDefaultsAndUniqueSubject,
    };
    match cohort {
        AgentEnabledAutonomousUnmanaged => {
            Box::pin(check_agent_enabled_autonomous_unmanaged(client))
        }
        BillingAgentAndKillSwitchNullable => {
            Box::pin(check_billing_agent_and_kill_switch_nullable(client))
        }
        IdentityActiveAndNullableRoleScopeDefaults => Box::pin(
            check_identity_active_and_nullable_role_scope_defaults(client),
        ),
        IdentityProviderTupleUnique => Box::pin(check_identity_provider_tuple_unique(client)),
        IssuerPrincipalUniqueWithNulls => {
            Box::pin(check_issuer_principal_unique_with_nulls(client))
        }
        LiveAndRetiredTupleUniquenessSeparate => {
            Box::pin(check_live_and_retired_tuple_uniqueness_separate(client))
        }
        RetiredTimestampNoAgentFk => Box::pin(check_retired_timestamp_no_agent_fk(client)),
        VerifiedHumanInteractiveDefaultsAndUniqueSubject => {
            Box::pin(check_verified_human_interactive_defaults_and_unique_subject(client))
        }
        TextDefaultsNotEnumChecks => Box::pin(check_text_defaults_not_enum_checks(client)),
        AgentDeleteLiveCascadeRetiredSetNull => {
            Box::pin(check_agent_delete_live_cascade_retired_set_null(client))
        }
        UserDeleteSubjectCascade => Box::pin(check_user_delete_subject_cascade(client)),
        IdentityParentUpdatesCascade => Box::pin(check_identity_parent_updates_cascade(client)),
        NoAutomaticRetiredRecord => Box::pin(check_no_automatic_retired_record(client)),
    }
}
async fn run_identity_defaults_check(client: &Client, cohort: IdentityDefaultsCohort) {
    apply_migrations_through(client, identity_defaults_checkpoint(cohort)).await;
    identity_defaults_check(client, cohort).await;
}
#[rstest]
#[case::agent_enabled_autonomous_unmanaged(IdentityDefaultsCohort::AgentEnabledAutonomousUnmanaged)]
#[case::billing_agent_and_kill_switch_nullable(
    IdentityDefaultsCohort::BillingAgentAndKillSwitchNullable
)]
#[case::identity_active_and_nullable_role_scope_defaults(
    IdentityDefaultsCohort::IdentityActiveAndNullableRoleScopeDefaults
)]
#[case::identity_provider_tuple_unique(IdentityDefaultsCohort::IdentityProviderTupleUnique)]
#[case::issuer_principal_unique_with_nulls(IdentityDefaultsCohort::IssuerPrincipalUniqueWithNulls)]
#[case::live_and_retired_tuple_uniqueness_separate(
    IdentityDefaultsCohort::LiveAndRetiredTupleUniquenessSeparate
)]
#[case::retired_timestamp_no_agent_fk(IdentityDefaultsCohort::RetiredTimestampNoAgentFk)]
#[case::verified_human_interactive_defaults_and_unique_subject(
    IdentityDefaultsCohort::VerifiedHumanInteractiveDefaultsAndUniqueSubject
)]
#[case::text_defaults_not_enum_checks(IdentityDefaultsCohort::TextDefaultsNotEnumChecks)]
#[case::agent_delete_live_cascade_retired_set_null(
    IdentityDefaultsCohort::AgentDeleteLiveCascadeRetiredSetNull
)]
#[case::user_delete_subject_cascade(IdentityDefaultsCohort::UserDeleteSubjectCascade)]
#[case::identity_parent_updates_cascade(IdentityDefaultsCohort::IdentityParentUpdatesCascade)]
#[case::no_automatic_retired_record(IdentityDefaultsCohort::NoAutomaticRetiredRecord)]
#[tokio::test]
async fn postgres_identity_defaults(#[case] cohort: IdentityDefaultsCohort) {
    Box::pin(harness(move |[client]| async move {
        run_identity_defaults_check(&client, cohort).await;
    }))
    .await;
}
