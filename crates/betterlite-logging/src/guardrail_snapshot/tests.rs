#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::callback_logger::event::GuardrailRequestSnapshot;

use super::{request_guardrail_fingerprint, request_mutating_guardrail_ran};

fn metadata(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the test metadata is a JSON object")
}

fn guardrail_history(entries: &str) -> IndexMap<String, Json> {
    metadata(&format!(
        r#"{{"standard_logging_guardrail_information": {entries}}}"#
    ))
}

fn request_body(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the test body is a JSON object")
}

#[test]
fn modes_no_guardrail_information_is_not_mutating() {
    assert!(!request_mutating_guardrail_ran(&metadata("{}")));
}

#[rstest]
#[case::object(r#"{"guardrail_mode": "pre_call"}"#)]
#[case::string(r#""pre_call""#)]
#[case::null("null")]
#[case::bool("true")]
#[case::number("7")]
#[case::empty_list("[]")]
fn modes_non_array_guardrail_information_has_no_entries(#[case] raw: &str) {
    assert!(!request_mutating_guardrail_ran(&guardrail_history(raw)));
}

#[rstest]
#[case::numbers("[1, 2]")]
#[case::strings(r#"["pre_call", "post_call"]"#)]
#[case::nulls("[null]")]
#[case::bools_and_lists(r#"[true, [], "pre_call"]"#)]
fn modes_non_object_entries_are_skipped(#[case] entries: &str) {
    assert!(!request_mutating_guardrail_ran(&guardrail_history(entries)));
}

#[rstest]
#[case::during_call("during_call")]
#[case::post_call("post_call")]
#[case::logging_only("logging_only")]
#[case::during_mcp_call("during_mcp_call")]
#[case::post_mcp_call("post_mcp_call")]
#[case::realtime_input_transcription("realtime_input_transcription")]
fn modes_each_non_mutating_mode_alone_is_not_mutating(#[case] mode: &str) {
    let entries = format!(r#"[{{"guardrail_mode": "{mode}"}}]"#);
    assert!(!request_mutating_guardrail_ran(&guardrail_history(
        &entries
    )));
}

#[rstest]
#[case::every_mode(r#"["during_call", "post_call", "logging_only", "during_mcp_call", "post_mcp_call", "realtime_input_transcription"]"#)]
#[case::no_mode("[]")]
fn modes_non_mutating_mode_arrays_are_not_mutating(#[case] modes: &str) {
    let entries = format!(r#"[{{"guardrail_mode": {modes}}}]"#);
    assert!(!request_mutating_guardrail_ran(&guardrail_history(
        &entries
    )));
}

#[rstest]
#[case::pre_call(r#"[{"guardrail_mode": "pre_call"}]"#)]
#[case::pre_mcp_call(r#"[{"guardrail_mode": "pre_mcp_call"}]"#)]
#[case::unknown_mode(r#"[{"guardrail_mode": "future_call"}]"#)]
#[case::missing_mode(r#"[{"guardrail_name": "pii"}]"#)]
#[case::null_mode(r#"[{"guardrail_mode": null}]"#)]
#[case::bool_mode(r#"[{"guardrail_mode": true}]"#)]
#[case::number_mode(r#"[{"guardrail_mode": 7}]"#)]
#[case::object_mode(r#"[{"guardrail_mode": {}}]"#)]
#[case::array_with_a_non_string(r#"[{"guardrail_mode": ["post_call", 7]}]"#)]
#[case::array_with_an_object(r#"[{"guardrail_mode": ["post_call", {}]}]"#)]
#[case::array_with_a_list(r#"[{"guardrail_mode": ["post_call", []]}]"#)]
#[case::array_with_a_mutating_mode(r#"[{"guardrail_mode": ["post_call", "pre_call"]}]"#)]
fn modes_mutating_entries_are_detected(#[case] entries: &str) {
    assert!(request_mutating_guardrail_ran(&guardrail_history(entries)));
}

#[test]
fn modes_one_mutating_entry_among_non_mutating_ones_is_detected() {
    let entries = r#"[{"guardrail_mode": "post_call"}, {"guardrail_mode": "pre_call"}, {"guardrail_mode": "during_call"}]"#;
    assert!(request_mutating_guardrail_ran(&guardrail_history(entries)));
}

#[test]
fn fingerprint_is_sha256_lowercase_hex() {
    let digest = request_guardrail_fingerprint(&guardrail_history("[]"))
        .expect("an empty guardrail history still hashes");
    assert_eq!(digest.len(), 64);
    assert!(
        digest
            .chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
    );
}

#[test]
fn fingerprint_of_no_entries_is_the_hash_of_the_empty_list() {
    let digest = request_guardrail_fingerprint(&metadata("{}"))
        .expect("a missing guardrail history still hashes");
    assert_eq!(
        digest,
        "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945"
    );
}

#[test]
fn fingerprint_ignores_object_key_order() {
    let first = guardrail_history(
        r#"[{"guardrail_mode": "pre_call", "guardrail_name": "pii", "nested": {"a": 1, "b": 2}, "policy": {"block": [{"kind": "email"}, {"kind": "phone"}]}}]"#,
    );
    let second = guardrail_history(
        r#"[{"policy": {"block": [{"kind": "email"}, {"kind": "phone"}]}, "nested": {"b": 2, "a": 1}, "guardrail_name": "pii", "guardrail_mode": "pre_call"}]"#,
    );
    assert_eq!(
        request_guardrail_fingerprint(&first),
        request_guardrail_fingerprint(&second)
    );
}

#[rstest]
#[case::post_call("post_call")]
#[case::during_call("during_call")]
#[case::during_mcp_call("during_mcp_call")]
#[case::post_mcp_call("post_mcp_call")]
#[case::realtime_input_transcription("realtime_input_transcription")]
fn fingerprint_ignores_a_post_call_entry(#[case] mode: &str) {
    let before = guardrail_history(r#"[{"guardrail_mode": "pre_call", "guardrail_name": "pii"}]"#);
    let after = guardrail_history(&format!(
        r#"[{{"guardrail_mode": "pre_call", "guardrail_name": "pii"}}, {{"guardrail_mode": "{mode}", "guardrail_name": "topic"}}]"#
    ));
    assert_eq!(
        request_guardrail_fingerprint(&before),
        request_guardrail_fingerprint(&after)
    );
}

#[test]
fn fingerprint_changes_with_a_logging_only_entry() {
    let before = guardrail_history(r#"[{"guardrail_mode": "pre_call", "guardrail_name": "pii"}]"#);
    let after = guardrail_history(
        r#"[{"guardrail_mode": "pre_call", "guardrail_name": "pii"}, {"guardrail_mode": "logging_only", "guardrail_name": "audit"}]"#,
    );
    assert_ne!(
        request_guardrail_fingerprint(&before),
        request_guardrail_fingerprint(&after)
    );
}

#[test]
fn fingerprint_changes_when_a_pre_call_entry_changes() {
    let before = guardrail_history(r#"[{"guardrail_mode": "pre_call", "guardrail_name": "pii"}]"#);
    let after = guardrail_history(r#"[{"guardrail_mode": "pre_call", "guardrail_name": "topic"}]"#);
    assert_ne!(
        request_guardrail_fingerprint(&before),
        request_guardrail_fingerprint(&after)
    );
}

#[test]
fn fingerprint_changes_with_entry_order() {
    let first = guardrail_history(
        r#"[{"guardrail_mode": "pre_call", "guardrail_name": "a"}, {"guardrail_mode": "pre_call", "guardrail_name": "b"}]"#,
    );
    let second = guardrail_history(
        r#"[{"guardrail_mode": "pre_call", "guardrail_name": "b"}, {"guardrail_mode": "pre_call", "guardrail_name": "a"}]"#,
    );
    assert_ne!(
        request_guardrail_fingerprint(&first),
        request_guardrail_fingerprint(&second)
    );
}

#[test]
fn fingerprint_missing_and_non_array_guardrail_information_agree() {
    let missing = metadata("{}");
    let not_an_array = guardrail_history(r#"{"guardrail_mode": "pre_call"}"#);
    let empty = guardrail_history("[]");
    assert_eq!(
        request_guardrail_fingerprint(&missing),
        request_guardrail_fingerprint(&empty)
    );
    assert_eq!(
        request_guardrail_fingerprint(&missing),
        request_guardrail_fingerprint(&not_an_array)
    );
}

#[rstest]
#[case::no_information("{}")]
#[case::during_call(r#"[{"guardrail_mode": "during_call"}]"#)]
#[case::post_call(r#"[{"guardrail_mode": "post_call"}]"#)]
#[case::logging_only(r#"[{"guardrail_mode": "logging_only"}]"#)]
fn capture_without_a_mutating_guardrail_is_none(#[case] entries: &str) {
    let captured = GuardrailRequestSnapshot::capture(
        &request_body(r#"{"model": "gpt-4o"}"#),
        &guardrail_history(entries),
    );
    assert!(captured.is_none(), "a non-mutating history has no snapshot");
}

#[test]
fn capture_keeps_the_body_and_the_fingerprint() {
    let history = guardrail_history(r#"[{"guardrail_mode": "pre_call"}]"#);
    let body = request_body(
        r#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}], "metadata": {"litellm_parent_otel_span": "otel-span"}, "litellm_metadata": {"litellm_parent_otel_span": "otel-span"}}"#,
    );
    let snapshot = GuardrailRequestSnapshot::capture(&body, &history)
        .expect("a pre_call guardrail ran, so a snapshot exists");
    assert_eq!(snapshot.body, body);
    assert_eq!(
        Some(snapshot.fingerprint),
        request_guardrail_fingerprint(&history)
    );
}

#[test]
fn capture_copies_the_body() {
    let history = guardrail_history(r#"[{"guardrail_mode": "pre_call"}]"#);
    let mut body = request_body(r#"{"model": "gpt-4o"}"#);
    let snapshot = GuardrailRequestSnapshot::capture(&body, &history)
        .expect("a pre_call guardrail ran, so a snapshot exists");
    body.insert("model".to_owned(), Json::Text("changed".to_owned()));
    assert_eq!(
        snapshot.body.get("model"),
        Some(&Json::Text("gpt-4o".to_owned()))
    );
}
