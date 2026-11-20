#![cfg(test)]

use betterlite_core::api::request_destination::RequestData;

use super::session_id_from_data;

fn data(wire: &str) -> RequestData {
    serde_json::from_str(wire).expect("the request body fixture decodes")
}

fn session(wire: &str) -> Option<String> {
    session_id_from_data(&data(wire))
}

#[test]
fn reads_the_top_level_litellm_session_id() {
    assert_eq!(
        session(r#"{"litellm_session_id": "sess-1"}"#),
        Some("sess-1".to_owned())
    );
}

#[test]
fn falls_back_to_metadata_session_id() {
    assert_eq!(
        session(r#"{"metadata": {"session_id": "sess-2"}}"#),
        Some("sess-2".to_owned())
    );
}

#[test]
fn falls_back_to_litellm_metadata_session_id() {
    assert_eq!(
        session(r#"{"litellm_metadata": {"session_id": "sess-3"}}"#),
        Some("sess-3".to_owned())
    );
}

#[test]
fn prefers_the_top_level_id_over_both_buckets() {
    let wire = r#"{
        "litellm_session_id": "top",
        "metadata": {"session_id": "meta"},
        "litellm_metadata": {"session_id": "llm"}
    }"#;
    assert_eq!(session(wire), Some("top".to_owned()));
}

#[test]
fn prefers_metadata_over_litellm_metadata() {
    let wire = r#"{
        "metadata": {"session_id": "meta"},
        "litellm_metadata": {"session_id": "llm"}
    }"#;
    assert_eq!(session(wire), Some("meta".to_owned()));
}

#[test]
fn uses_litellm_metadata_when_metadata_has_no_session_id() {
    let wire = r#"{
        "metadata": {"user_id": "user-1"},
        "litellm_metadata": {"session_id": "llm"}
    }"#;
    assert_eq!(session(wire), Some("llm".to_owned()));
}

#[test]
fn returns_none_when_no_source_names_a_session() {
    assert_eq!(session(r#"{"metadata": {"user_id": "user-1"}}"#), None);
}

#[test]
fn renders_a_numeric_session_id_as_text() {
    assert_eq!(
        session(r#"{"litellm_session_id": 7}"#),
        Some("7".to_owned())
    );
}

#[test]
fn keeps_a_zero_session_id() {
    assert_eq!(
        session(r#"{"metadata": {"session_id": 0}}"#),
        Some("0".to_owned())
    );
}

#[test]
fn keeps_an_explicit_empty_session_id_as_given() {
    assert_eq!(
        session(r#"{"metadata": {"session_id": ""}}"#),
        Some(String::new())
    );
}

#[test]
fn ignores_a_bool_session_id() {
    assert_eq!(session(r#"{"metadata": {"session_id": true}}"#), None);
}

#[test]
fn ignores_a_null_session_id_and_falls_through() {
    let wire = r#"{
        "metadata": {"session_id": null},
        "litellm_metadata": {"session_id": "llm"}
    }"#;
    assert_eq!(session(wire), Some("llm".to_owned()));
}

#[test]
fn ignores_non_object_metadata() {
    let wire = r#"{
        "metadata": "sess-text",
        "litellm_metadata": {"session_id": "llm"}
    }"#;
    assert_eq!(session(wire), Some("llm".to_owned()));
}

#[test]
fn ignores_a_session_id_list() {
    assert_eq!(session(r#"{"litellm_session_id": ["s"]}"#), None);
}
