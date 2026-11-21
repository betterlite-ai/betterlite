#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use super::AgentPersistenceError;
use crate::agent_persistence::error::APPLICATION_CONFLICT;
use crate::agent_persistence::support::Database;
use crate::agent_persistence::support::IDENTITY;
use crate::agent_persistence::support::RecordingPublisher;
use crate::agent_persistence::support::agent;
use crate::agent_persistence::support::patch;
use crate::agent_persistence::support::store_with;

const FULL_AGENT: &str = r#"{"agent_name": "a", "litellm_params": {"api_key": "sk-real"}, "static_headers": {"x": "1"}, "extra_headers": ["old"], "tpm_limit": 3}"#;
const HEADERS_SQL: &str =
    r#"SELECT to_jsonb(static_headers)::text FROM "LiteLLM_AgentsTable" WHERE agent_id = $1"#;
const PARAMS_SQL: &str =
    r#"SELECT to_jsonb(litellm_params)::text FROM "LiteLLM_AgentsTable" WHERE agent_id = $1"#;
const SERVERS_SQL: &str = r#"SELECT to_jsonb(mcp_servers)::text FROM "LiteLLM_ObjectPermissionTable" WHERE object_permission_id = $1"#;
const MODELS_SQL: &str = r#"SELECT to_jsonb(models)::text FROM "LiteLLM_ObjectPermissionTable" WHERE object_permission_id = $1"#;
const REVISION_SQL: &str = r#"SELECT revision FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#;
const ACTIVE_SQL: &str = r#"SELECT active::text FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#;
const CLEARED_SQL: &str = r#"SELECT (last_authenticated_at IS NULL)::text FROM "LiteLLM_AgentIdentity" WHERE agent_id = $1"#;

#[tokio::test]
async fn patch_updates_the_carried_fields_and_publishes_once() {
    let database = Database::start().await;
    let publisher = RecordingPublisher::default();
    let store = store_with(&database, publisher.clone());
    let created = store
        .add_agent_to_db(&agent(FULL_AGENT), "author", None)
        .await
        .expect("the agent is created");
    let request = patch(r#"{"agent_name": "b", "static_headers": null}"#);
    let patched = store
        .patch_agent_in_db(&created.agent_id, &request, "editor")
        .await
        .expect("the agent is patched");
    assert_eq!(patched.agent_name, "b");
    assert_eq!(patched.created_by.as_deref(), Some("author"));
    assert_eq!(patched.updated_by.as_deref(), Some("editor"));
    let headers = database.json(HEADERS_SQL, &[&created.agent_id]).await;
    assert_eq!(headers, Some(JsonData::Object(BTreeMap::new())));
    let decoded = database.decode_agent(&created.agent_id).await;
    assert_eq!(decoded.agent.tpm_limit, Some(3));
    let params = database.json(PARAMS_SQL, &[&created.agent_id]).await;
    assert_eq!(
        params,
        Some(JsonData::Object(BTreeMap::from([(
            String::from("api_key"),
            JsonData::String(String::from("sk-real"))
        )])))
    );
    assert_eq!(publisher.calls(), 2);
    database.close().await;
}

#[tokio::test]
async fn patch_reports_a_missing_agent_with_its_own_text() {
    let database = Database::start().await;
    let publisher = RecordingPublisher::default();
    let store = store_with(&database, publisher.clone());
    let request = patch(r#"{"agent_name": "b"}"#);
    let failure = store
        .patch_agent_in_db("missing", &request, "editor")
        .await
        .expect_err("the agent does not exist");
    let AgentPersistenceError::NotFound(text) = failure else {
        panic!("unexpected failure: {failure}");
    };
    assert_eq!(text, "Agent with ID missing not found");
    assert_eq!(publisher.calls(), 0);
    database.close().await;
}

#[tokio::test]
async fn patch_links_a_permission_row_and_reuses_it_on_the_next_patch() {
    let database = Database::start().await;
    let publisher = RecordingPublisher::default();
    let store = store_with(&database, publisher.clone());
    let created = store
        .add_agent_to_db(&agent(r#"{"agent_name": "a"}"#), "author", None)
        .await
        .expect("the agent is created");
    let first = patch(r#"{"object_permission": {"mcp_servers": ["srv"]}}"#);
    store
        .patch_agent_in_db(&created.agent_id, &first, "editor")
        .await
        .expect("the permission row is linked");
    let linked = database.decode_agent(&created.agent_id).await;
    let permission_id = linked
        .object_permission_id
        .expect("the agent links a permission row");
    let second = patch(r#"{"object_permission": {"models": ["gpt-4o"]}}"#);
    store
        .patch_agent_in_db(&created.agent_id, &second, "editor")
        .await
        .expect("the permission row is updated");
    let updated = database.decode_agent(&created.agent_id).await;
    assert_eq!(
        updated.object_permission_id.as_deref(),
        Some(permission_id.as_str())
    );
    let servers = database.json(SERVERS_SQL, &[&permission_id]).await;
    assert_eq!(
        servers,
        Some(JsonData::Array(vec![JsonData::String(String::from("srv"))]))
    );
    let models = database.json(MODELS_SQL, &[&permission_id]).await;
    assert_eq!(
        models,
        Some(JsonData::Array(vec![JsonData::String(String::from(
            "gpt-4o"
        ))]))
    );
    assert_eq!(publisher.calls(), 3);
    database.close().await;
}

#[tokio::test]
async fn patch_rejects_a_retired_identity_of_another_agent() {
    let database = Database::start().await;
    let publisher = RecordingPublisher::default();
    let store = store_with(&database, publisher.clone());
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
    let created = store
        .add_agent_to_db(&agent(r#"{"agent_name": "a"}"#), "author", None)
        .await
        .expect("the agent is created");
    let request = patch(&format!("{{{IDENTITY}}}"));
    let failure = store
        .patch_agent_in_db(&created.agent_id, &request, "editor")
        .await
        .expect_err("the application belongs to another agent");
    let AgentPersistenceError::Conflict(text) = failure else {
        panic!("unexpected failure: {failure}");
    };
    assert_eq!(text, APPLICATION_CONFLICT);
    assert_eq!(publisher.calls(), 1);
    database.close().await;
}

#[tokio::test]
async fn patch_unbinds_an_identity_and_refuses_to_enable_it_again() {
    let database = Database::start().await;
    let publisher = RecordingPublisher::default();
    let store = store_with(&database, publisher.clone());
    let config = agent(&format!(r#"{{"agent_name": "a", {IDENTITY}}}"#));
    let created = store
        .add_agent_to_db(&config, "author", None)
        .await
        .expect("the agent is created");
    let agent_id = created.agent_id.as_str();
    let revision = database
        .text(REVISION_SQL, &[&agent_id])
        .await
        .expect("the identity row exists");
    let unbind = patch(r#"{"identity": null}"#);
    store
        .patch_agent_in_db(agent_id, &unbind, "editor")
        .await
        .expect("the identity is unbound");
    let inactive = database.text(ACTIVE_SQL, &[&agent_id]).await;
    assert_eq!(inactive.as_deref(), Some("false"));
    let stamped = database.text(CLEARED_SQL, &[&agent_id]).await;
    assert_eq!(stamped.as_deref(), Some("true"));
    let changed = database.text(REVISION_SQL, &[&agent_id]).await;
    assert_ne!(changed, Some(revision));
    let enable = patch(r#"{"enabled": true}"#);
    let refusal = store
        .patch_agent_in_db(agent_id, &enable, "editor")
        .await
        .expect_err("an unbound managed agent cannot be enabled");
    let AgentPersistenceError::Identity(denied) = refusal else {
        panic!("unexpected failure: {refusal}");
    };
    assert_eq!(
        denied.message,
        "Bind an identity before enabling this managed agent"
    );
    assert_eq!(publisher.calls(), 2);
    database.close().await;
}
