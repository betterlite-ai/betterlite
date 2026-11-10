use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{object, value};
use crate::python_json;
use crate::spend_bodies::classifier::{
    SnapshotValue, classifier_audit_fields, classifier_input_snapshot, decode_classifier_snapshot,
    without_classifier_audit,
};
use crate::spend_bodies::support::Fields;
use crate::spend_sanitize::value::{NodeId, SpendMapping, SpendValue};

fn snapshot(text: &str) -> Option<String> {
    classifier_input_snapshot(SnapshotValue::Text(text), false)
        .map(|stored| python_json::dumps(&stored))
}

fn sdk_snapshot(value: &SpendValue) -> Option<String> {
    classifier_input_snapshot(SnapshotValue::Value(value), true)
        .map(|stored| python_json::dumps(&stored))
}

fn audit(entries: &Fields) -> String {
    python_json::dumps(&Json::Object(entries.clone()))
}

#[test]
fn snapshots_a_json_object_string() {
    assert_eq!(
        snapshot(r#"{"model": "gpt-4o", "messages": []}"#),
        Some(r#"{"model": "gpt-4o", "messages": []}"#.to_owned())
    );
    let bytes = decode_classifier_snapshot(br#"{"a": 1}"#);
    assert!(bytes.is_some());
}

#[rstest::rstest]
#[case::array("[1, 2]")]
#[case::scalar("\"text\"")]
#[case::invalid("{not json")]
#[case::empty("")]
fn rejects_a_json_value_that_is_not_an_object(#[case] text: &str) {
    assert_eq!(snapshot(text), None);
}

#[test]
fn rejects_bytes_that_are_not_utf8() {
    assert!(classifier_input_snapshot(SnapshotValue::Bytes(b"\xff\xfe"), false).is_none());
}

#[test]
fn openai_sdk_drops_transport_keys_and_merges_extra_body() {
    let sdk_input = value(
        r#"{"model": "gpt-4o", "extra_headers": {"a": "b"}, "extra_query": {"q": 1}, "extra_body": {"temperature": 1, "model": "override"}}"#,
    );
    assert_eq!(
        sdk_snapshot(&SpendValue::from(&sdk_input)),
        Some(r#"{"model": "override", "temperature": 1}"#.to_owned())
    );
    let without_mapping = value(r#"{"model": "gpt-4o", "extra_body": "not a mapping"}"#);
    assert_eq!(
        sdk_snapshot(&SpendValue::from(&without_mapping)),
        Some(r#"{"model": "gpt-4o"}"#.to_owned())
    );
}

#[test]
fn a_plain_snapshot_keeps_the_transport_keys() {
    let source = value(r#"{"model": "gpt-4o", "extra_body": {"temperature": 1}}"#);
    let snapshot =
        classifier_input_snapshot(SnapshotValue::Value(&SpendValue::from(&source)), false)
            .expect("plain object");
    assert_eq!(
        python_json::dumps(&snapshot),
        r#"{"model": "gpt-4o", "extra_body": {"temperature": 1}}"#
    );
}

#[test]
fn rejects_foreign_objects_in_a_mapping() {
    let model = SpendValue::Model(SpendMapping {
        node: NodeId::from(1_u64),
        entries: IndexMap::from([("content".to_owned(), SpendValue::Text("x".to_owned()))]),
    });
    let source = SpendValue::Mapping(SpendMapping {
        node: NodeId::from(2_u64),
        entries: IndexMap::from([("classifier_input".to_owned(), model)]),
    });
    assert!(classifier_input_snapshot(SnapshotValue::Value(&source), false).is_none());
}

#[rstest::rstest]
#[case::both(
    r#"{"classifier_input": {"prompt": "pick"}, "originating_request_masked": {"messages": []}, "trace_id": "t"}"#,
    r#"{"classifier_input": {"prompt": "pick"}, "originating_request_masked": {"messages": []}}"#
)]
#[case::classifier_only(
    r#"{"classifier_input": "{\"prompt\": \"pick\"}", "originating_request_masked": "not json"}"#,
    r#"{"classifier_input": {"prompt": "pick"}}"#
)]
#[case::originating_only(
    r#"{"classifier_input": [], "originating_request_masked": {"a": 1}}"#,
    r#"{"originating_request_masked": {"a": 1}}"#
)]
#[case::neither(r#"{"trace_id": "t"}"#, "{}")]
fn classifier_audit_fields_keep_only_valid_entries(#[case] payload: &str, #[case] expected: &str) {
    let decoded = object(payload);
    assert_eq!(audit(&classifier_audit_fields(&decoded)), expected);
}

#[test]
fn without_classifier_audit_removes_only_those_two_keys() {
    let payload = object(
        r#"{"trace_id": "t", "classifier_input": {"a": 1}, "model": "m", "originating_request_masked": {"b": 2}}"#,
    );
    let stripped = without_classifier_audit(&payload);
    assert_eq!(
        python_json::dumps(&Json::Object(stripped)),
        r#"{"trace_id": "t", "model": "m"}"#
    );
}
