#![cfg(test)]

use std::str;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::schema_agent_identity;
use betterlite_db::schema_baseline;
use betterlite_db::schema_budget_windows;
use betterlite_db::schema_daily_usage;
use betterlite_db::schema_identity_history;
use betterlite_db::schema_managed_resources;
use betterlite_db::schema_policy_resources;
use betterlite_db::schema_routing_state;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::SecretString;
use tokio_postgres::NoTls;

use crate::agent_identity::error::IDENTITY_DENIED_STATUS;
use crate::agent_policy::AgentExecutionMode;
use crate::agent_store::AgentIdentityStore;
use crate::agent_store::resolve::resolve_managed_agent;
use crate::agent_store::tests::support::{
    application_claims, authentication_context, microsoft_subject,
};
use crate::identity::FailureCode;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, AgentIdentityStore, PostgresStore) {
    let url = Environment::parse_from(["agent-store-repository-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .expect("the database fixture");
    let pool = Pool::builder(Manager::new(fixture.options.config.clone(), NoTls))
        .max_size(2)
        .build()
        .expect("the database pool");
    let postgres = PostgresStore { pool: Some(pool) };
    apply_schema(&postgres).await;
    let store = AgentIdentityStore::from_client(&postgres, None);
    (fixture, store, postgres)
}

// the migration catalogs the schema contract test treats together as the current schema
async fn apply_schema(postgres: &PostgresStore) {
    let client = postgres.session().await.expect("the database connection");
    for catalog in [
        schema_baseline::MIGRATION_CATALOG,
        schema_daily_usage::MIGRATION_CATALOG,
        schema_managed_resources::MIGRATION_CATALOG,
        schema_identity_history::MIGRATION_CATALOG,
        schema_policy_resources::MIGRATION_CATALOG,
        schema_routing_state::MIGRATION_CATALOG,
        schema_budget_windows::MIGRATION_CATALOG,
        schema_agent_identity::MIGRATION_CATALOG,
    ] {
        for migration in catalog.files {
            let sql = str::from_utf8(migration.bytes).expect("the migration text");
            client
                .batch_execute(sql)
                .await
                .expect("the schema migration");
        }
    }
}

async fn close(fixture: PostgresFixture, postgres: PostgresStore) {
    if let Some(pool) = postgres.pool {
        pool.close();
    }
    fixture.close_async().await.expect("the fixture teardown");
}

async fn insert_agent(client: &Client, agent_id: &str) {
    let agent_name = format!("{agent_id} name");
    let permission_id = format!("{agent_id} permission");
    client
        .execute(
            r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id, mcp_servers)
               VALUES ($1, ARRAY['mcp-1'])"#,
            &[&permission_id],
        )
        .await
        .expect("the permission row");
    client
        .execute(
            r#"INSERT INTO "LiteLLM_AgentsTable"
               (agent_id, agent_name, agent_card_params, created_by, updated_by,
                identity_managed, enabled, execution_mode, spend, object_permission_id)
               VALUES ($1, $2, '{}', 'tester', 'tester', true, true, 'autonomous', 1.5, $3)"#,
            &[&agent_id, &agent_name, &permission_id],
        )
        .await
        .expect("the agent row");
}

async fn insert_identity(client: &Client, agent_id: &str, client_id: &str, revision: &str) {
    let service_principal_id = format!("{agent_id} principal");
    client
        .execute(
            r#"INSERT INTO "LiteLLM_AgentIdentity"
               (agent_id, provider, issuer, tenant_id, client_id, service_principal_id, revision)
               VALUES ($1, 'microsoft_entra', 'issuer-1', 'tenant-1', $2, $3, $4)"#,
            &[&agent_id, &client_id, &service_principal_id, &revision],
        )
        .await
        .expect("the identity row");
}

async fn insert_retired_identity(client: &Client, binding_id: &str, client_id: &str) {
    client
        .execute(
            r#"INSERT INTO "LiteLLM_RetiredAgentIdentity"
               (binding_id, provider, issuer, tenant_id, client_id)
               VALUES ($1, 'microsoft_entra', 'issuer-1', 'tenant-1', $2)"#,
            &[&binding_id, &client_id],
        )
        .await
        .expect("the retired identity row");
}

async fn insert_retired_agent(client: &Client, agent_id: &str) {
    client
        .execute(
            r#"INSERT INTO "LiteLLM_RetiredAgent" (original_agent_id) VALUES ($1)"#,
            &[&agent_id],
        )
        .await
        .expect("the retired agent row");
}

async fn insert_user(client: &Client, user_id: &str) {
    client
        .execute(
            r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ($1)"#,
            &[&user_id],
        )
        .await
        .expect("the user row");
}

async fn disable(client: &Client, agent_id: &str, column: &str) {
    let sql = format!(r#"UPDATE "LiteLLM_AgentsTable" SET {column} = false WHERE agent_id = $1"#);
    client
        .execute(&sql, &[&agent_id])
        .await
        .expect("the disabled agent");
}

#[tokio::test]
async fn repository_agent_row_keeps_its_identity_binding() {
    let (fixture, store, postgres) = database().await;
    {
        let client = postgres.session().await.expect("the database connection");
        insert_agent(&client, "agent-1").await;
        insert_identity(&client, "agent-1", "client-1", "rev-1").await;
    }
    let found = store.agent("agent-1").await.expect("the agent read");
    let agent = found.expect("the stored agent");
    assert_eq!(agent.agent_id, "agent-1");
    assert!(agent.identity_managed);
    assert_eq!(agent.execution_mode, AgentExecutionMode::Autonomous);
    let binding = agent.identity.expect("the included identity");
    assert_eq!(binding.client_id, "client-1");
    assert_eq!(binding.revision, "rev-1");
    assert_eq!(
        binding.required_scopes.0,
        vec![String::from("user_impersonation")]
    );
    let permission = agent.object_permission.expect("the included permission");
    assert!(permission.contains_key("mcp_servers"));
    assert!(agent.spend.is_some());
    close(fixture, postgres).await;
}

#[tokio::test]
async fn repository_binding_reads_the_composite_key() {
    let (fixture, store, postgres) = database().await;
    {
        let client = postgres.session().await.expect("the database connection");
        insert_agent(&client, "agent-1").await;
        insert_identity(&client, "agent-1", "client-1", "rev-1").await;
    }
    let bound = store
        .bound_agent_id("tenant-1", "client-1")
        .await
        .expect("the binding read");
    assert_eq!(bound.as_deref(), Some("agent-1"));
    let other_tenant = store
        .bound_agent_id("tenant-2", "client-1")
        .await
        .expect("the binding read");
    assert!(other_tenant.is_none());
    let missing = store.agent("agent-9").await.expect("the agent read");
    assert!(missing.is_none());
    close(fixture, postgres).await;
}

#[tokio::test]
async fn repository_retired_binding_and_history_are_visible() {
    let (fixture, store, postgres) = database().await;
    {
        let client = postgres.session().await.expect("the database connection");
        insert_retired_identity(&client, "binding-1", "client-2").await;
        insert_retired_agent(&client, "agent-9").await;
    }
    let failure = store
        .bound_agent_id("tenant-1", "client-2")
        .await
        .expect_err("the retired binding");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "This agent identity binding has been retired"
    );
    assert!(
        store
            .retired_agent("agent-9")
            .await
            .expect("the history read")
    );
    assert!(
        !store
            .retired_agent("agent-1")
            .await
            .expect("the history read")
    );
    close(fixture, postgres).await;
}

#[tokio::test]
async fn repository_authentication_update_counts_the_active_revision() {
    let (fixture, store, postgres) = database().await;
    {
        let client = postgres.session().await.expect("the database connection");
        insert_agent(&client, "agent-1").await;
        insert_identity(&client, "agent-1", "client-1", "rev-1").await;
    }
    store
        .record_authentication(&authentication_context("agent-1", Some("rev-1")))
        .await
        .expect("the recorded authentication");
    {
        let client = postgres.session().await.expect("the database connection");
        let recorded: bool = client
            .query_one(
                r#"SELECT last_authenticated_at IS NOT NULL
                   FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
                &[&"agent-1"],
            )
            .await
            .expect("the stored instant")
            .get(0);
        assert!(recorded);
    }
    let failure = store
        .record_authentication(&authentication_context("agent-1", Some("rev-0")))
        .await
        .expect_err("the stale revision");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "Agent identity changed during authentication; retry"
    );
    close(fixture, postgres).await;
}

#[tokio::test]
async fn repository_authentication_update_excludes_a_disabled_agent() {
    let (fixture, store, postgres) = database().await;
    {
        let client = postgres.session().await.expect("the database connection");
        insert_agent(&client, "agent-1").await;
        insert_identity(&client, "agent-1", "client-1", "rev-1").await;
        insert_agent(&client, "agent-2").await;
        insert_identity(&client, "agent-2", "client-2", "rev-2").await;
        disable(&client, "agent-1", "enabled").await;
        disable(&client, "agent-2", "identity_managed").await;
    }
    for (agent_id, revision) in [("agent-1", "rev-1"), ("agent-2", "rev-2")] {
        let failure = store
            .record_authentication(&authentication_context(agent_id, Some(revision)))
            .await
            .expect_err("the filtered agent");
        assert_eq!(failure.code, FailureCode::IdentityDenied);
        assert_eq!(
            failure.message,
            "Agent identity changed during authentication; retry"
        );
    }
    close(fixture, postgres).await;
}

#[tokio::test]
async fn repository_enrollment_create_data_keeps_the_existing_row() {
    let (fixture, store, postgres) = database().await;
    {
        let client = postgres.session().await.expect("the database connection");
        insert_user(&client, "user-1").await;
    }
    store
        .enroll_interactive_human(&microsoft_subject(), "user-1")
        .await
        .expect("the enrollment");
    let oid = JsonData::String(String::from("oid-2"));
    let row = store
        .subject("issuer-1", "tenant-1", Some(&oid))
        .await
        .expect("the subject read")
        .expect("the enrolled subject");
    assert_eq!(row.user_id.as_deref(), Some("user-1"));
    assert_eq!(row.kind, "human");
    assert_eq!(row.verified_via, "sso_interactive");
    assert!(!row.subject_id.is_empty());
    store
        .enroll_interactive_human(&microsoft_subject(), "user-1")
        .await
        .expect("the repeated enrollment");
    let failure = store
        .enroll_interactive_human(&microsoft_subject(), "user-2")
        .await
        .expect_err("the other user");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "Microsoft subject is already bound to another local identity"
    );
    close(fixture, postgres).await;
}

#[tokio::test]
async fn repository_resolve_managed_agent_denies_a_disabled_agent() {
    let (fixture, _store, postgres) = database().await;
    {
        let client = postgres.session().await.expect("the database connection");
        insert_agent(&client, "agent-1").await;
        insert_identity(&client, "agent-1", "client-1", "rev-1").await;
        disable(&client, "agent-1", "enabled").await;
    }
    let failure = resolve_managed_agent(&application_claims(), Some(&postgres), None)
        .await
        .expect_err("the disabled agent");
    assert_eq!(failure.status, IDENTITY_DENIED_STATUS);
    assert_eq!(
        failure.message,
        "Agent is disabled or no longer bound to an identity"
    );
    close(fixture, postgres).await;
}
