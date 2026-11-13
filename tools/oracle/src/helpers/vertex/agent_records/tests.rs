#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;

fn run(value: Value) -> Value {
    let input = serde_json::from_value(value).unwrap();
    super::ops::run(&input).unwrap()
}

#[test]
fn helper_seeds_select_vertex_agent_records() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_agent_records"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        seed.call.operation == "vertex_agent_records"
            && seed
                .call
                .input
                .as_object()
                .is_some_and(|input| !input.is_empty())
    }));
}

#[test]
fn an_unknown_input_field_is_rejected() {
    let unknown =
        serde_json::from_value::<super::ops::Input>(json!({"operation": "url", "other": 1_i32}));
    assert!(unknown.unwrap_err().to_string().contains("other"));
}

#[test]
fn a_url_call_returns_the_production_stream_query_url() {
    let value = run(json!({
        "operation": "url",
        "calls": [{
            "model": "agent_engine/7",
            "project": "proj",
            "location": "europe-west4",
        }],
    }));
    assert_eq!(
        value["calls"][0]["url"],
        json!(
            "https://europe-west4-aiplatform.googleapis.com/v1beta1/projects/proj/locations/europe-west4/reasoningEngines/7:streamQuery"
        )
    );
}

#[test]
fn a_url_call_without_a_project_reports_no_url() {
    let value = run(json!({
        "operation": "url",
        "calls": [{"model": "agent_engine/7", "project": null, "location": null}],
    }));
    assert_eq!(value["calls"][0]["ok"], json!(false));
}

#[test]
fn a_request_call_maps_user_and_keeps_the_last_message() {
    let value = run(json!({
        "operation": "request",
        "calls": [{
            "messages": [
                {"role": "user", "content": "first"},
                {"role": "user", "content": "second"},
            ],
            "non_default_params": {"user": "ada"},
            "optional_params": {"session_id": ""},
            "uuid": "123e4567-e89b-12d3-a456-426614174000",
        }],
    }));
    let payload = &value["calls"][0]["payload"];
    assert_eq!(payload["class_method"], json!("stream_query"));
    assert_eq!(payload["input"]["message"], json!("second"));
    assert_eq!(payload["input"]["user_id"], json!("ada"));
    assert_eq!(payload["input"].get("session_id"), None);
}

#[test]
fn a_response_call_collects_the_last_snapshot() {
    let value = run(json!({
        "operation": "response",
        "calls": [{
            "status": 200,
            "body": "{\"content\":{\"parts\":[{\"text\":\"first\"}]}}\n{\"content\":{\"parts\":[{\"text\":\"last\"}]}}",
            "model": "agent_engine/7",
            "messages": [{"role": "user", "content": "hi"}],
            "prompt_tokens": 3,
            "completion_tokens": 2,
            "counter": "fixed",
        }],
    }));
    let answer = &value["calls"][0]["answer"];
    assert_eq!(answer["choice"]["message"]["content"], json!("last"));
    assert_eq!(answer["choice"]["finish_reason"], json!("stop"));
    assert_eq!(answer["choice"]["index"], json!(0));
    assert_eq!(answer["usage"], json!([3, 2, 5]));
}

#[test]
fn a_failing_counter_leaves_the_usage_null() {
    let value = run(json!({
        "operation": "response",
        "calls": [{
            "status": 200,
            "body": "{\"content\":{\"parts\":[{\"text\":\"kept\"}]}}",
            "model": "agent_engine/7",
            "messages": [{"role": "user", "content": "hi"}],
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "counter": "failure",
        }],
    }));
    assert_eq!(value["calls"][0]["answer"]["usage"], Value::Null);
    assert_eq!(
        value["calls"][0]["answer"]["choice"]["message"]["content"],
        json!("kept")
    );
}
