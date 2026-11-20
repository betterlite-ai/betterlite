#![cfg(test)]

use betterlite_auth::agent_identity::write_fields::IdentityFields;
use betterlite_auth::agent_identity::write_fields::IdentityHistoryEntry;
use betterlite_auth::agent_identity::write_fields::IdentityHistoryKey;
use betterlite_auth::agent_identity::write_fields::IdentityHistoryWrite;
use betterlite_auth::agent_identity::write_fields::IdentityRelationWrite;
use betterlite_auth::agent_identity::write_fields::IdentityUpsert;
use betterlite_db::budget_permission_rows::JsonData;
use chrono::Utc;

use super::history_should_insert;
use super::write_history;
use super::write_identity;
use crate::agent_persistence::error::APPLICATION_CONFLICT;
use crate::agent_persistence::error::AgentPersistenceError;
use crate::agent_persistence::support::Database;

async fn seed_agent(database: &Database, agent_id: &str) {
    database
        .execute(
            r#"INSERT INTO "LiteLLM_AgentsTable" (agent_id, agent_name, agent_card_params, created_by, updated_by) VALUES ($1, $2, '{}'::jsonb, $3, $3)"#,
            &[&agent_id, &agent_id, &"seed"],
        )
        .await;
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
    let mut client = store.session().await.expect("a session");
    let tx = client.transaction().await.expect("a transaction");
    write_identity(&tx, "agent-1", &IdentityRelationWrite::Create(fields()))
        .await
        .expect("the identity row is written");
    tx.commit().await.expect("the transaction commits");
    let provider = database
        .text(
            r#"SELECT provider FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
        )
        .await;
    assert_eq!(provider.as_deref(), Some("microsoft_entra"));
    let principal = database
        .text(
            r#"SELECT service_principal_id FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
        )
        .await;
    assert_eq!(principal.as_deref(), Some("sp-1"));
    let roles = database
        .json(
            r#"SELECT to_jsonb(required_roles)::text FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
        )
        .await;
    assert_eq!(
        roles,
        Some(JsonData::Array(vec![JsonData::String(String::from(
            "admin"
        ))]))
    );
    let active = database
        .text(
            r#"SELECT active::text FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
        )
        .await;
    assert_eq!(active.as_deref(), Some("true"));
    let stamped = database
        .text(
            r#"SELECT (last_authenticated_at IS NOT NULL)::text FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
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
    let mut client = store.session().await.expect("a session");
    let tx = client.transaction().await.expect("a transaction");
    write_identity(&tx, "agent-1", &IdentityRelationWrite::Create(fields()))
        .await
        .expect("the identity row is written");
    tx.commit().await.expect("the transaction commits");
    let bound = IdentityFields {
        provider: None,
        tenant_id: None,
        client_id: None,
        issuer: None,
        service_principal_id: None,
        required_roles: None,
        required_scopes: None,
        active: Some(false),
        revision: Some(String::from("r2")),
        last_authenticated_at: Some(None),
    };
    let mut updating = store.session().await.expect("a session");
    let updating_tx = updating.transaction().await.expect("a transaction");
    write_identity(
        &updating_tx,
        "agent-1",
        &IdentityRelationWrite::Update(bound),
    )
    .await
    .expect("the identity row is updated");
    updating_tx.commit().await.expect("the transaction commits");
    let active = database
        .text(
            r#"SELECT active::text FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
        )
        .await;
    assert_eq!(active.as_deref(), Some("false"));
    let revision = database
        .text(
            r#"SELECT revision FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
        )
        .await;
    assert_eq!(revision.as_deref(), Some("r2"));
    let issuer = database
        .text(
            r#"SELECT issuer FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
        )
        .await;
    assert_eq!(issuer.as_deref(), Some("issuer-1"));
    let principal = database
        .text(
            r#"SELECT service_principal_id FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
        )
        .await;
    assert_eq!(principal.as_deref(), Some("sp-1"));
    let stamped = database
        .text(
            r#"SELECT (last_authenticated_at IS NULL)::text FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
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
    let mut client = store.session().await.expect("a session");
    let tx = client.transaction().await.expect("a transaction");
    write_identity(&tx, "agent-1", &IdentityRelationWrite::Create(fields()))
        .await
        .expect("the identity row is written");
    tx.commit().await.expect("the transaction commits");
    let mut untouched = store.session().await.expect("a session");
    let untouched_tx = untouched.transaction().await.expect("a transaction");
    write_identity(
        &untouched_tx,
        "agent-1",
        &IdentityRelationWrite::Update(IdentityFields::default()),
    )
    .await
    .expect("the identity row is left alone");
    untouched_tx
        .commit()
        .await
        .expect("the transaction commits");
    let revision = database
        .text(
            r#"SELECT revision FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
        )
        .await;
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
            provider: None,
            tenant_id: None,
            client_id: None,
            issuer: None,
            service_principal_id: None,
            required_roles: None,
            required_scopes: None,
            active: Some(false),
            revision: Some(String::from("r2")),
            last_authenticated_at: None,
        },
    };
    let first = IdentityRelationWrite::Upsert(Box::new(upsert.clone()));
    let mut client = store.session().await.expect("a session");
    let tx = client.transaction().await.expect("a transaction");
    write_identity(&tx, "agent-1", &first)
        .await
        .expect("the identity row is inserted");
    tx.commit().await.expect("the transaction commits");
    let active = database
        .text(
            r#"SELECT active::text FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
        )
        .await;
    assert_eq!(active.as_deref(), Some("true"));
    let second = IdentityRelationWrite::Upsert(Box::new(upsert));
    let mut upserting = store.session().await.expect("a session");
    let upserting_tx = upserting.transaction().await.expect("a transaction");
    write_identity(&upserting_tx, "agent-1", &second)
        .await
        .expect("the identity row is updated");
    upserting_tx
        .commit()
        .await
        .expect("the transaction commits");
    let flipped = database
        .text(
            r#"SELECT active::text FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
        )
        .await;
    assert_eq!(flipped.as_deref(), Some("false"));
    let revision = database
        .text(
            r#"SELECT revision FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
        )
        .await;
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
    let mut client = store.session().await.expect("a session");
    let tx = client.transaction().await.expect("a transaction");
    write_identity(&tx, "agent-1", &relation)
        .await
        .expect("nothing is written");
    tx.commit().await.expect("the transaction commits");
    assert_eq!(
        database
            .count(
                r#"SELECT count(*)::bigint FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
                &[&"agent-1"],
            )
            .await,
        0
    );
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
    let mut client = store.session().await.expect("a session");
    let tx = client.transaction().await.expect("a transaction");
    write_history(&tx, Some(&entry), "agent-1", true)
        .await
        .expect("the history row is written");
    tx.commit().await.expect("the transaction commits");
    let binding = database
        .text(
            r#"SELECT binding_id FROM "LiteLLM_RetiredAgentIdentity" WHERE agent_id = $1"#,
            &[&"agent-1"],
        )
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
    match failure {
        AgentPersistenceError::Conflict(text) => assert_eq!(text, APPLICATION_CONFLICT),
        other => panic!("unexpected failure: {other}"),
    }
    let mut suppressing = store.session().await.expect("a session");
    let suppressing_tx = suppressing.transaction().await.expect("a transaction");
    write_history(&suppressing_tx, Some(&entry), "agent-1", false)
        .await
        .expect("the suppressed history row is skipped");
    suppressing_tx
        .commit()
        .await
        .expect("the transaction commits");
    assert_eq!(
        database
            .count(
                r#"SELECT count(*)::bigint FROM "LiteLLM_RetiredAgentIdentity""#,
                &[]
            )
            .await,
        1
    );
    database.close().await;
}
