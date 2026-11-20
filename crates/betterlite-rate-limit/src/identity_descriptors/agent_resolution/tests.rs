#![cfg(test)]

use betterlite_core::api::request_destination::RequestData;

use super::resolved_agent_id;

fn data(wire: &str) -> RequestData {
    serde_json::from_str(wire).expect("the request body fixture decodes")
}

#[test]
fn key_agent_id_wins_over_metadata() {
    let body = data(r#"{"metadata": {"agent_id": "meta-agent"}}"#);
    assert_eq!(
        resolved_agent_id(Some("key-agent"), &body),
        Some("key-agent")
    );
}

#[test]
fn metadata_names_the_agent_without_a_key_agent() {
    let body = data(r#"{"metadata": {"agent_id": "meta-agent"}}"#);
    assert_eq!(resolved_agent_id(None, &body), Some("meta-agent"));
}

#[test]
fn returns_none_when_neither_names_an_agent() {
    let body = data(r#"{"metadata": {"user_id": "user-1"}}"#);
    assert_eq!(resolved_agent_id(None, &body), None);
}

#[test]
fn ignores_a_non_text_metadata_agent_id() {
    let body = data(r#"{"metadata": {"agent_id": 5}}"#);
    assert_eq!(resolved_agent_id(None, &body), None);
}

#[test]
fn ignores_a_null_metadata_agent_id() {
    let body = data(r#"{"metadata": {"agent_id": null}}"#);
    assert_eq!(resolved_agent_id(None, &body), None);
}

#[test]
fn ignores_non_object_metadata() {
    let body = data(r#"{"metadata": "agent-1"}"#);
    assert_eq!(resolved_agent_id(None, &body), None);
}

#[test]
fn ignores_agent_ids_outside_metadata() {
    let body = data(r#"{"agent_id": "top-agent"}"#);
    assert_eq!(resolved_agent_id(None, &body), None);
}
