#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;

use super::AgentPersistenceError;
use crate::agent_persistence::error::APPLICATION_CONFLICT;
use crate::agent_persistence::error::REGISTRATION_CONFLICT;
use crate::agent_persistence::support::Database;
use crate::agent_persistence::support::IDENTITY;
use crate::agent_persistence::support::RecordingPublisher;
use crate::agent_persistence::support::agent;
use crate::agent_persistence::support::store_with;

#[tokio::test]
async fn create_stores_the_agent_permission_identity_and_history() {
    let database = Database::start().await;
    let publisher = RecordingPublisher::default();
    let store = store_with(&database, publisher.clone());
    let config = agent(&format!(
        r#"{{"agent_name": "a", {IDENTITY}, "object_permission": {{"mcp_servers": ["srv"]}}}}"#
    ));
    let created = store
        .add_agent_to_db(&config, "author", None)
        .await
        .expect("the agent is created");
    assert_eq!(created.agent_name, "a");
    assert_eq!(created.created_by.as_deref(), Some("author"));
    assert_eq!(publisher.calls(), 1);
    let decoded = database.decode_agent(&created.agent_id).await;
    assert!(decoded.agent.identity_managed);
    assert!(decoded.object_permission_id.is_some());
    let identities = database
        .count(
            r#"SELECT count(*)::bigint FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#,
            &[&created.agent_id],
        )
        .await;
    assert_eq!(identities, 1);
    let history = database
        .count(
            r#"SELECT count(*)::bigint FROM "LiteLLM_RetiredAgentIdentity" WHERE agent_id = $1"#,
            &[&created.agent_id],
        )
        .await;
    assert_eq!(history, 1);
    let permission = database
        .json(
            r#"SELECT to_jsonb(mcp_servers)::text FROM "LiteLLM_ObjectPermissionTable" WHERE object_permission_id = $1"#,
            &[&decoded.object_permission_id.expect("a linked permission")],
        )
        .await;
    assert_eq!(
        permission,
        Some(JsonData::Array(vec![JsonData::String(String::from("srv"))]))
    );
    database.close().await;
}

#[tokio::test]
async fn create_honours_a_caller_agent_id() {
    let database = Database::start().await;
    let store = store_with(&database, RecordingPublisher::default());
    let config = agent(r#"{"agent_name": "a"}"#);
    let created = store
        .add_agent_to_db(&config, "author", Some("chosen-id"))
        .await
        .expect("the agent is created");
    assert_eq!(created.agent_id, "chosen-id");
    database.close().await;
}

#[tokio::test]
async fn create_reports_a_duplicate_agent_name_as_conflict() {
    let database = Database::start().await;
    let store = store_with(&database, RecordingPublisher::default());
    let config = agent(r#"{"agent_name": "a"}"#);
    store
        .add_agent_to_db(&config, "author", None)
        .await
        .expect("the first agent is created");
    let failure = store
        .add_agent_to_db(&config, "author", None)
        .await
        .expect_err("the second agent conflicts");
    match failure {
        AgentPersistenceError::Conflict(text) => assert_eq!(text, REGISTRATION_CONFLICT),
        other => panic!("unexpected failure: {other}"),
    }
    assert_eq!(
        database
            .count(r#"SELECT count(*)::bigint FROM "LiteLLM_AgentsTable""#, &[])
            .await,
        1
    );
    database.close().await;
}

#[tokio::test]
async fn create_rolls_back_the_permission_row_with_a_failed_agent_write() {
    let database = Database::start().await;
    let store = store_with(&database, RecordingPublisher::default());
    let config = agent(r#"{"agent_name": "a"}"#);
    store
        .add_agent_to_db(&config, "author", None)
        .await
        .expect("the first agent is created");
    let duplicate = agent(r#"{"agent_name": "a", "object_permission": {"mcp_servers": ["srv"]}}"#);
    store
        .add_agent_to_db(&duplicate, "author", None)
        .await
        .expect_err("the duplicate agent conflicts");
    assert_eq!(
        database
            .count(
                r#"SELECT count(*)::bigint FROM "LiteLLM_ObjectPermissionTable""#,
                &[]
            )
            .await,
        0
    );
    database.close().await;
}

#[tokio::test]
async fn create_reports_a_history_row_of_another_agent_as_conflict() {
    let database = Database::start().await;
    let store = store_with(&database, RecordingPublisher::default());
    database
        .run(
            r#"INSERT INTO "LiteLLM_AgentsTable" (agent_id, agent_name, agent_card_params, created_by, updated_by) VALUES ('other-agent', 'other-agent', '{}'::jsonb, 'seed', 'seed')"#,
        )
        .await;
    database
        .run(
            r#"INSERT INTO "LiteLLM_RetiredAgentIdentity" (binding_id, agent_id, provider, issuer, tenant_id, client_id) VALUES ('binding-kept', 'other-agent', 'microsoft_entra', 'issuer-old', '11111111-1111-1111-1111-111111111111', '22222222-2222-2222-2222-222222222222')"#,
        )
        .await;
    let config = agent(&format!(r#"{{"agent_name": "a", {IDENTITY}}}"#));
    let failure = store
        .add_agent_to_db(&config, "author", None)
        .await
        .expect_err("the application belongs to another agent");
    match failure {
        AgentPersistenceError::Conflict(text) => assert_eq!(text, APPLICATION_CONFLICT),
        other => panic!("unexpected failure: {other}"),
    }
    database.close().await;
}

#[tokio::test]
async fn create_reports_an_unbound_history_row_as_conflict() {
    let database = Database::start().await;
    let store = store_with(&database, RecordingPublisher::default());
    database
        .run(
            r#"INSERT INTO "LiteLLM_RetiredAgentIdentity" (binding_id, agent_id, provider, issuer, tenant_id, client_id) VALUES ('binding-free', NULL, 'microsoft_entra', 'issuer-old', '11111111-1111-1111-1111-111111111111', '22222222-2222-2222-2222-222222222222')"#,
        )
        .await;
    let config = agent(&format!(r#"{{"agent_name": "a", {IDENTITY}}}"#));
    let failure = store
        .add_agent_to_db(&config, "author", None)
        .await
        .expect_err("the application belonged to a deleted agent");
    match failure {
        AgentPersistenceError::Conflict(text) => assert_eq!(text, APPLICATION_CONFLICT),
        other => panic!("unexpected failure: {other}"),
    }
    database.close().await;
}

#[tokio::test]
async fn create_does_not_publish_after_a_failed_write() {
    let database = Database::start().await;
    let publisher = RecordingPublisher::default();
    let store = store_with(&database, publisher.clone());
    let config = agent(r#"{"agent_name": "a"}"#);
    store
        .add_agent_to_db(&config, "author", None)
        .await
        .expect("the first agent is created");
    store
        .add_agent_to_db(&config, "author", None)
        .await
        .expect_err("the duplicate agent conflicts");
    assert_eq!(publisher.calls(), 1);
    database.close().await;
}
