#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::sqs_logging::tests::support::payload;

use super::item;

const KEYS: [&str; 11] = [
    "id",
    "call_type",
    "startTime",
    "endTime",
    "model",
    "user",
    "modelParameters",
    "messages",
    "response",
    "usage",
    "metadata",
];

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

#[test]
fn carries_the_litellm_payload_keys() {
    let item = item(&payload()).expect("the item builds");
    let keys: Vec<&str> = item.keys().map(String::as_str).collect();
    assert_eq!(keys, KEYS);
    assert_eq!(item["id"], "sqs-health-check");
    assert_eq!(item["call_type"], "completion");
    assert_eq!(item["model"], "gpt-3.5-turbo");
}

#[test]
fn formats_the_timestamps_as_rfc3339() {
    let item = item(&payload()).expect("the item builds");
    assert_eq!(item["startTime"], "2009-02-13T23:31:30+00:00");
    assert_eq!(item["endTime"], "2009-02-13T23:31:31+00:00");
}

#[test]
fn stringifies_nested_structures() {
    let mut standard = payload();
    standard.messages = Some(json(r#"[{"role":"user","content":"hi"}]"#));
    let mut parameters = IndexMap::new();
    parameters.insert("temperature".to_owned(), json("0.5"));
    standard.model_parameters = parameters;
    standard.response = Some(json(r#"{"id":"chatcmpl-1"}"#));
    let item = item(&standard).expect("the item builds");
    assert_eq!(item["messages"], r#"[{"role":"user","content":"hi"}]"#);
    assert_eq!(item["modelParameters"], r#"{"temperature":0.5}"#);
    assert_eq!(item["response"], r#"{"id":"chatcmpl-1"}"#);
}

#[test]
fn writes_null_for_an_absent_message_and_response() {
    let item = item(&payload()).expect("the item builds");
    assert_eq!(item["messages"], "null");
    assert_eq!(item["response"], "null");
    assert_eq!(item["usage"], "null");
}

#[test]
fn uses_the_response_usage() {
    let mut standard = payload();
    standard.response = Some(json(r#"{"usage":{"total_tokens":30}}"#));
    standard.metadata.usage_object = Some(json(r#"{"total_tokens":1}"#));
    let item = item(&standard).expect("the item builds");
    assert_eq!(item["usage"], r#"{"total_tokens":30}"#);
}

#[test]
fn falls_back_to_the_metadata_usage() {
    let mut standard = payload();
    standard.metadata.usage_object = Some(json(r#"{"total_tokens":1}"#));
    let item = item(&standard).expect("the item builds");
    assert_eq!(item["usage"], r#"{"total_tokens":1}"#);
}

#[test]
fn carries_the_end_user() {
    let mut standard = payload();
    standard.end_user = Some("user-1".to_owned());
    let item = item(&standard).expect("the item builds");
    assert_eq!(item["user"], "user-1");
}

#[test]
fn leaves_the_user_empty_when_the_payload_has_none() {
    let item = item(&payload()).expect("the item builds");
    assert_eq!(item["user"], "");
}

#[test]
fn writes_the_metadata_as_json() {
    let mut standard = payload();
    standard.metadata.user_api_key_team_alias = Some("team".to_owned());
    let item = item(&standard).expect("the item builds");
    let Json::Object(fields) = json(&item["metadata"]) else {
        panic!("the metadata is an object");
    };
    assert_eq!(
        fields.get("user_api_key_team_alias"),
        Some(&Json::Text("team".to_owned()))
    );
}
