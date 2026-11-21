#![cfg(test)]

use betterlite_auth::agent_identity::write_fields::IdentityFields;
use betterlite_auth::agent_identity::write_fields::IdentityHistoryEntry;
use betterlite_auth::agent_identity::write_fields::IdentityHistoryKey;
use betterlite_auth::agent_identity::write_fields::IdentityHistoryWrite;
use betterlite_auth::agent_identity::write_fields::IdentityRelationWrite;
use betterlite_auth::agent_identity::write_fields::IdentityUpsert;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::Utc;

use super::history_should_insert;
use super::write_history;
use super::write_identity;
use crate::agent_persistence::error::APPLICATION_CONFLICT;
use crate::agent_persistence::error::AgentPersistenceError;
use crate::agent_persistence::support::Database;
use crate::agent_persistence::support::count_where;
use crate::agent_persistence::support::history_column_text;
use crate::agent_persistence::support::identity_column_json;
use crate::agent_persistence::support::identity_column_text;

const IDENTITIES: &str = "LiteLLM_AgentIdentity";
const RETIRED: &str = "LiteLLM_RetiredAgentIdentity";

async fn seed_agent(database: &Database, agent_id: &str) {
    database
        .execute(
            r#"INSERT INTO "LiteLLM_AgentsTable" (agent_id, agent_name, agent_card_params, created_by, updated_by) VALUES ($1, $2, '{}'::jsonb, $3, $3)"#,
            &[&agent_id, &agent_id, &"seed"],
        )
        .await;
}

async fn write_relation(store: &PostgresStore, agent_id: &str, relation: &IdentityRelationWrite) {
    let mut client = store.session().await.expect("a session");
    let tx = client.transaction().await.expect("a transaction");
    write_identity(&tx, agent_id, relation)
        .await
        .expect("the identity row is written");
    tx.commit().await.expect("the transaction commits");
}

async fn write_history_row(
    store: &PostgresStore,
    entry: &IdentityHistoryWrite,
    agent_id: &str,
    insert: bool,
) {
    let mut client = store.session().await.expect("a session");
    let tx = client.transaction().await.expect("a transaction");
    write_history(&tx, Some(entry), agent_id, insert)
        .await
        .expect("the history row is handled");
    tx.commit().await.expect("the transaction commits");
}

fn fields() -> IdentityFields {
    IdentityFields {
        provider: Some(String::from("microsoft_entra")),
        tenant_id: Some(String::from("tenant-1")),
        client_id: Some(String::from("client-1")),
        issuer: Some(String::from("issuer-1")),
        service_principal_id: Some(Some(String::from("sp-1"))),
        required_roles: Some(vec![String::from("admin")]),
        required_scopes: Some(vec![String::from("scope")]),
        active: Some(true),
        revision: Some(String::from("r1")),
        last_authenticated_at: Some(Some(Utc::now())),
    }
}

fn history() -> IdentityHistoryWrite {
    IdentityHistoryWrite {
        create: IdentityHistoryEntry {
            key: IdentityHistoryKey {
                provider: String::from("microsoft_entra"),
                tenant_id: String::from("tenant-1"),
                client_id: String::from("client-1"),
            },
            issuer: String::from("issuer-1"),
        },
    }
}

#[tokio::test]
async fn identity_create_inserts_every_carried_field() {
    let database = Database::start().await;
    seed_agent(&database, "agent-1").await;
    let store = database.store();
    write_relation(&store, "agent-1", &IdentityRelationWrite::Create(fields())).await;
    let provider = identity_column_text(&database, "provider", "agent-1").await;
    assert_eq!(provider.as_deref(), Some("microsoft_entra"));
    let principal = identity_column_text(&database, "service_principal_id", "agent-1").await;
    assert_eq!(principal.as_deref(), Some("sp-1"));
    let roles = identity_column_json(&database, "required_roles", "agent-1").await;
    assert_eq!(
        roles,
        Some(JsonData::Array(vec![JsonData::String(String::from(
            "admin"
        ))]))
    );
    let active = identity_column_text(&database, "active::text", "agent-1").await;
    assert_eq!(active.as_deref(), Some("true"));
    let stamped = identity_column_text(
        &database,
        "(last_authenticated_at IS NOT NULL)::text",
        "agent-1",
    )
    .await;
    assert_eq!(stamped.as_deref(), Some("true"));
    database.close().await;
}

#[tokio::test]
async fn identity_update_changes_only_the_carried_columns() {
    let database = Database::start().await;
    seed_agent(&database, "agent-1").await;
    let store = database.store();
    write_relation(&store, "agent-1", &IdentityRelationWrite::Create(fields())).await;
    let bound = IdentityFields {
        active: Some(false),
        revision: Some(String::from("r2")),
        last_authenticated_at: Some(None),
        ..IdentityFields::default()
    };
    write_relation(&store, "agent-1", &IdentityRelationWrite::Update(bound)).await;
    let active = identity_column_text(&database, "active::text", "agent-1").await;
    assert_eq!(active.as_deref(), Some("false"));
    let revision = identity_column_text(&database, "revision", "agent-1").await;
    assert_eq!(revision.as_deref(), Some("r2"));
    let issuer = identity_column_text(&database, "issuer", "agent-1").await;
    assert_eq!(issuer.as_deref(), Some("issuer-1"));
    let principal = identity_column_text(&database, "service_principal_id", "agent-1").await;
    assert_eq!(principal.as_deref(), Some("sp-1"));
    let stamped = identity_column_text(
        &database,
        "(last_authenticated_at IS NULL)::text",
        "agent-1",
    )
    .await;
    assert_eq!(stamped.as_deref(), Some("true"));
    database.close().await;
}

#[tokio::test]
async fn identity_update_with_no_fields_leaves_the_row_intact() {
    let database = Database::start().await;
    seed_agent(&database, "agent-1").await;
    let store = database.store();
    write_relation(&store, "agent-1", &IdentityRelationWrite::Create(fields())).await;
    write_relation(
        &store,
        "agent-1",
        &IdentityRelationWrite::Update(IdentityFields::default()),
    )
    .await;
    let revision = identity_column_text(&database, "revision", "agent-1").await;
    assert_eq!(revision.as_deref(), Some("r1"));
    database.close().await;
}

#[tokio::test]
async fn identity_upsert_creates_then_updates_the_row() {
    let database = Database::start().await;
    seed_agent(&database, "agent-1").await;
    let store = database.store();
    let upsert = IdentityUpsert {
        create: fields(),
        update: IdentityFields {
            active: Some(false),
            revision: Some(String::from("r2")),
            ..IdentityFields::default()
        },
    };
    let relation = IdentityRelationWrite::Upsert(Box::new(upsert));
    write_relation(&store, "agent-1", &relation).await;
    let active = identity_column_text(&database, "active::text", "agent-1").await;
    assert_eq!(active.as_deref(), Some("true"));
    write_relation(&store, "agent-1", &relation).await;
    let flipped = identity_column_text(&database, "active::text", "agent-1").await;
    assert_eq!(flipped.as_deref(), Some("false"));
    let revision = identity_column_text(&database, "revision", "agent-1").await;
    assert_eq!(revision.as_deref(), Some("r2"));
    database.close().await;
}

#[tokio::test]
async fn identity_upsert_with_an_empty_update_writes_nothing() {
    let database = Database::start().await;
    seed_agent(&database, "agent-1").await;
    let store = database.store();
    let upsert = IdentityUpsert {
        create: fields(),
        update: IdentityFields::default(),
    };
    let relation = IdentityRelationWrite::Upsert(Box::new(upsert));
    write_relation(&store, "agent-1", &relation).await;
    let rows = count_where(&database, IDENTITIES, "agent_id", "agent-1").await;
    assert_eq!(rows, 0);
    database.close().await;
}

#[tokio::test]
async fn history_is_inserted_once_for_the_bound_agent_only() {
    let database = Database::start().await;
    seed_agent(&database, "agent-1").await;
    let store = database.store();
    let entry = history();
    assert!(
        history_should_insert(&store, Some(&entry), None)
            .await
            .expect("a free application")
    );
    assert!(
        !history_should_insert(&store, None, None)
            .await
            .expect("no history")
    );
    write_history_row(&store, &entry, "agent-1", true).await;
    let binding = history_column_text(&database, "binding_id", "agent-1")
        .await
        .expect("the history row exists");
    assert_eq!(binding.len(), 36);
    assert!(
        !history_should_insert(&store, Some(&entry), Some("agent-1"))
            .await
            .expect("the same agent keeps its row")
    );
    let failure = history_should_insert(&store, Some(&entry), Some("agent-2"))
        .await
        .expect_err("the application belongs to another agent");
    let AgentPersistenceError::Conflict(text) = failure else {
        panic!("unexpected failure: {failure}");
    };
    assert_eq!(text, APPLICATION_CONFLICT);
    write_history_row(&store, &entry, "agent-1", false).await;
    let rows = count_where(&database, RETIRED, "agent_id", "agent-1").await;
    assert_eq!(rows, 1);
    database.close().await;
}
