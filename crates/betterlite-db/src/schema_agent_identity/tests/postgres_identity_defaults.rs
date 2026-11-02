#[derive(Clone, Copy)]
enum IdentityDefaultsCohort {
    AgentEnabledAutonomousUnmanaged,
    BillingAgentAndKillSwitchNullable,
    IdentityActiveAndNullableRoleScopeDefaults,
    IdentityProviderTupleUnique,
    IssuerPrincipalUniqueWithNulls,
    LiveAndRetiredTupleUniquenessSeparate,
    RetiredTimestampNoAgentFk,
    VerifiedHumanInteractiveDefaultsAndUniqueSubject,
    TextDefaultsNotEnumChecks,
    AgentDeleteLiveCascadeRetiredSetNull,
    UserDeleteSubjectCascade,
    IdentityParentUpdatesCascade,
    NoAutomaticRetiredRecord,
}
async fn insert_agent(client: &Client, id: &str) {
    exec_params(client, r#"INSERT INTO "LiteLLM_AgentsTable" (agent_id, agent_name, agent_card_params, created_by, updated_by) VALUES ($1, $1, '{}', $2, $2)"#, &[&id, &"operator"]).await;
}
async fn insert_identity(client: &Client, agent_id: &str, provider: &str, tuple: &str) {
    exec_params(client, r#"INSERT INTO "LiteLLM_AgentIdentity" (agent_id, provider, issuer, tenant_id, client_id, revision) VALUES ($1, $2, $3, $3, $3, 'rev-1')"#, &[&agent_id, &provider, &tuple]).await;
}
async fn check_agent_enabled_autonomous_unmanaged(client: &Client) {
    insert_agent(client, "agent-a").await;
    assert_columns(
        client,
        "LiteLLM_AgentsTable",
        &[
            bool_true("enabled"),
            text_default("execution_mode", "'autonomous'::text"),
            bool_false("identity_managed"),
        ],
    )
    .await;
}
async fn check_billing_agent_and_kill_switch_nullable(client: &Client) {
    insert_agent(client, "agent-b").await;
    assert_column(client, "LiteLLM_AgentsTable", nullable_jsonb("kill_switch")).await;
    batch_execute(
        client,
        r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, "startTime", "endTime") VALUES ('spend-a', 'completion', now(), now())"#,
    )
    .await;
    assert_column(
        client,
        "LiteLLM_SpendLogs",
        nullable_text("billing_agent_id"),
    )
    .await;
}
async fn check_identity_active_and_nullable_role_scope_defaults(client: &Client) {
    insert_agent(client, "agent-c").await;
    insert_identity(client, "agent-c", "azure-ad", "tuple-c").await;
    assert_columns(
        client,
        "LiteLLM_AgentIdentity",
        &[
            bool_true("active"),
            nullable_text_array("required_roles", "ARRAY[]::text[]"),
            nullable_text_array("required_scopes", "ARRAY['user_impersonation'::text]"),
        ],
    )
    .await;
}
async fn check_identity_provider_tuple_unique(client: &Client) {
    insert_agent(client, "agent-d1").await;
    insert_agent(client, "agent-d2").await;
    insert_identity(client, "agent-d1", "okta", "tuple-d").await;
    assert_exec_params_err_state(
        client,
        r#"INSERT INTO "LiteLLM_AgentIdentity" (agent_id, provider, issuer, tenant_id, client_id, revision) VALUES ($1, 'okta', 'tuple-d-other', 'tuple-d', 'tuple-d', 'rev-1')"#,
        &[&"agent-d2"],
        SqlState::UNIQUE_VIOLATION,
    )
    .await;
}
async fn check_issuer_principal_unique_with_nulls(client: &Client) {
    for suffix in ["e1", "e2", "e3", "e4"] {
        insert_agent(client, &format!("agent-{suffix}")).await;
    }
    for (id, principal) in [
        ("agent-e1", None),
        ("agent-e2", None),
        ("agent-e3", Some("sp-1")),
    ] {
        exec_params(
            client,
            r#"INSERT INTO "LiteLLM_AgentIdentity" (agent_id, provider, issuer, tenant_id, client_id, service_principal_id, revision) VALUES ($1, $1, 'issuer-e', $1, $1, $2, 'rev-1')"#,
            &[&id, &principal],
        )
        .await;
    }
    assert_exec_params_err_state(
        client,
        r#"INSERT INTO "LiteLLM_AgentIdentity" (agent_id, provider, issuer, tenant_id, client_id, service_principal_id, revision) VALUES ('agent-e4', 'agent-e4', 'issuer-e', 'agent-e4', 'agent-e4', 'sp-1', 'rev-1')"#,
        &[],
        SqlState::UNIQUE_VIOLATION,
    )
    .await;
}
async fn check_live_and_retired_tuple_uniqueness_separate(client: &Client) {
    insert_agent(client, "agent-f").await;
    insert_identity(client, "agent-f", "github", "tuple-f").await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_RetiredAgentIdentity" (binding_id, provider, issuer, tenant_id, client_id) VALUES ($1, 'github', 'tuple-f', 'tuple-f', 'tuple-f')"#,
        &[&"binding-f"],
    )
    .await;
    let provider = scalar_text(
        client,
        r#"SELECT provider FROM "LiteLLM_RetiredAgentIdentity" WHERE binding_id = $1"#,
        &[&"binding-f"],
    )
    .await;
    assert_eq!(provider, "github");
}
async fn check_retired_timestamp_no_agent_fk(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_RetiredAgent" (original_agent_id) VALUES ($1)"#,
        &[&"agent-never-existed"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_RetiredAgent",
        timestamp3("retired_at", Some("CURRENT_TIMESTAMP")),
    )
    .await;
}
async fn check_verified_human_interactive_defaults_and_unique_subject(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ($1)"#,
        &[&"user-g"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerifiedSubject" (subject_id, issuer, tenant_id, oid, user_id) VALUES ('subject-g1', 'issuer-g', 'tenant-g', 'oid-g', $1)"#,
        &[&"user-g"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_VerifiedSubject",
        &[
            text_default("kind", "'human'::text"),
            text_default("verified_via", "'sso_interactive'::text"),
            timestamp3("verified_at", Some("CURRENT_TIMESTAMP")),
        ],
    )
    .await;
    assert_exec_params_err_state(
        client,
        r#"INSERT INTO "LiteLLM_VerifiedSubject" (subject_id, issuer, tenant_id, oid) VALUES ('subject-g2', 'issuer-g', 'tenant-g', 'oid-g')"#,
        &[],
        SqlState::UNIQUE_VIOLATION,
    )
    .await;
}
async fn check_text_defaults_not_enum_checks(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AgentsTable" (agent_id, agent_name, agent_card_params, created_by, updated_by, execution_mode) VALUES ($1, $1, '{}', $2, $2, $3)"#,
        &[&"agent-h", &"operator", &"custom-mode"],
    )
    .await;
    let execution_mode = scalar_text(
        client,
        r#"SELECT execution_mode FROM "LiteLLM_AgentsTable" WHERE agent_id = $1"#,
        &[&"agent-h"],
    )
    .await;
    assert_eq!(execution_mode, "custom-mode");
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerifiedSubject" (subject_id, issuer, tenant_id, oid, kind, verified_via) VALUES ('subject-h', 'issuer-h', 'tenant-h', 'oid-h', 'anything-goes', 'anything-too')"#,
        &[],
    )
    .await;
    let kind = scalar_text(
        client,
        r#"SELECT kind FROM "LiteLLM_VerifiedSubject" WHERE subject_id = $1"#,
        &[&"subject-h"],
    )
    .await;
    assert_eq!(kind, "anything-goes");
}
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
async fn run_identity_defaults_check(client: &Client, cohort: IdentityDefaultsCohort) {
    use IdentityDefaultsCohort::*;
    apply_migrations_through(client, identity_defaults_checkpoint(cohort)).await;
    let check: Pin<Box<dyn Future<Output = ()> + Send + '_>> = match cohort {
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
    };
    check.await;
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
