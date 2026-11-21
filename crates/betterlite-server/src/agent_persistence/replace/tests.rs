#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use crate::agent_credentials::REDACTED_BY_LITELM;
use crate::agent_persistence::error::AgentPersistenceError;
use crate::agent_persistence::support::Database;
use crate::agent_persistence::support::RecordingPublisher;
use crate::agent_persistence::support::agent;
use crate::agent_persistence::support::agent_column_json;
use crate::agent_persistence::support::agent_column_text;
use crate::agent_persistence::support::store_with;

const FULL_AGENT: &str = r#"{"agent_name": "a", "agent_card_params": {}, "litellm_params": {"api_key": "sk-real"}, "static_headers": {"x": "1"}, "extra_headers": ["old"], "access_group_ids": ["team"], "tpm_limit": 3, "kill_switch": {"url": "https://example.test/hook", "auth": {"type": "bearer", "token": "s3cret"}}}"#;
const EXTRA_HEADERS: &str = "cardinality(extra_headers)::text";
const ACCESS_GROUPS: &str = "cardinality(access_group_ids)::text";

fn object_field(value: Option<JsonData>, name: &str) -> Option<JsonData> {
    match value {
        Some(JsonData::Object(fields)) => fields.get(name).cloned(),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Array(_),
        )
        | None => None,
    }
}

#[tokio::test]
async fn replace_restores_a_marker_secret_and_clears_omitted_columns() {
    let database = Database::start().await;
    let publisher = RecordingPublisher::default();
    let store = store_with(&database, publisher.clone());
    let created = store
        .add_agent_to_db(&agent(FULL_AGENT), "author", None)
        .await
        .expect("the agent is created");
    let put = agent(&format!(
        r#"{{"agent_name": "a", "agent_card_params": {{}}, "litellm_params": {{"api_key": "{REDACTED_BY_LITELM}", "model": "m"}}}}"#
    ));
    store
        .update_agent_in_db(&created.agent_id, &put, "editor")
        .await
        .expect("the agent is replaced");
    let params = agent_column_json(&database, "litellm_params", &created.agent_id).await;
    assert_eq!(
        object_field(params, "api_key"),
        Some(JsonData::String(String::from("sk-real")))
    );
    let headers = agent_column_json(&database, "static_headers", &created.agent_id).await;
    assert_eq!(headers, Some(JsonData::Object(BTreeMap::new())));
    let groups = agent_column_text(&database, EXTRA_HEADERS, &created.agent_id).await;
    assert_eq!(groups.as_deref(), Some("0"));
    let access = agent_column_text(&database, ACCESS_GROUPS, &created.agent_id).await;
    assert_eq!(access.as_deref(), Some("0"));
    let switch = agent_column_json(&database, "kill_switch", &created.agent_id).await;
    assert_eq!(switch, Some(JsonData::Null));
    let limit = agent_column_text(&database, "tpm_limit::text", &created.agent_id).await;
    assert_eq!(limit.as_deref(), Some("3"));
    assert_eq!(publisher.calls(), 2);
    database.close().await;
}

#[tokio::test]
async fn replace_reports_a_missing_agent() {
    let database = Database::start().await;
    let store = store_with(&database, RecordingPublisher::default());
    let put = agent(r#"{"agent_name": "a", "agent_card_params": {}}"#);
    let failure = store
        .update_agent_in_db("missing", &put, "editor")
        .await
        .expect_err("the agent does not exist");
    let AgentPersistenceError::NotFound(text) = failure else {
        panic!("unexpected failure: {failure}");
    };
    assert_eq!(text, "Agent not found, passed agent_id=missing");
    database.close().await;
}

#[tokio::test]
async fn replace_without_a_card_behaves_as_a_patch() {
    let database = Database::start().await;
    let publisher = RecordingPublisher::default();
    let store = store_with(&database, publisher.clone());
    let created = store
        .add_agent_to_db(&agent(FULL_AGENT), "author", None)
        .await
        .expect("the agent is created");
    let put = agent(r#"{"agent_name": "a", "static_headers": {"y": "2"}}"#);
    store
        .update_agent_in_db(&created.agent_id, &put, "editor")
        .await
        .expect("the agent is patched through the replace");
    let extra = agent_column_text(&database, "extra_headers::text", &created.agent_id).await;
    assert_eq!(extra.as_deref(), Some("{old}"));
    let headers = agent_column_json(&database, "static_headers", &created.agent_id).await;
    assert_eq!(
        object_field(headers, "y"),
        Some(JsonData::String(String::from("2")))
    );
    assert_eq!(publisher.calls(), 2);
    database.close().await;
}
