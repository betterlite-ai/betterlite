#![cfg(test)]

mod deferral;
mod model_guardrails;
mod streaming;

use std::collections::BTreeSet;
use std::fs::read_to_string;
use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_policy::pipeline::{GuardrailPipeline, PipelineMode};
use rstest::rstest;
use serde_json::json;
use tracing::Level;
use tracing::subscriber::with_default;

use super::slots::{
    is_pending_background_response, merge_pipeline_metadata_writes, pipeline_managed_names,
    policy_pipelines, policy_state_metadata, post_call_pipelines, step_guardrail_names,
};

fn capture_warnings<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(Level::DEBUG)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, run);
    (result, read_to_string(log.path()).unwrap())
}

fn object(entries: Vec<(&str, JsonValue)>) -> JsonMap {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn pipeline(mode: PipelineMode, guardrails: &[&str]) -> GuardrailPipeline {
    let steps: Vec<JsonValue> = guardrails
        .iter()
        .map(|guardrail| json!({"guardrail": guardrail}))
        .collect();
    serde_json::from_value(json!({"mode": mode, "steps": steps})).unwrap()
}

fn pipeline_pairs(pipelines: &[(&str, GuardrailPipeline)]) -> Vec<JsonValue> {
    pipelines
        .iter()
        .map(|(policy_name, pipeline)| json!([policy_name, pipeline]))
        .collect()
}

fn pipeline_bucket(pipelines: &[(&str, GuardrailPipeline)]) -> JsonValue {
    json!({"_guardrail_pipelines": pipeline_pairs(pipelines)})
}

fn data_with(pipelines: &[(&str, GuardrailPipeline)]) -> JsonMap {
    object(vec![("metadata", pipeline_bucket(pipelines))])
}

#[rstest]
#[case::metadata_slot(
    json!({"metadata": {"custom": 1, "_pipeline_managed_guardrails": ["pii"]}}),
    Some(json!({"custom": 1, "_pipeline_managed_guardrails": ["pii"]}))
)]
#[case::litellm_metadata_slot(
    json!({"metadata": {"custom": 1}, "litellm_metadata": {"_guardrail_pipelines": []}}),
    Some(json!({"_guardrail_pipelines": []}))
)]
#[case::metadata_first_when_both_qualify(
    json!({"metadata": {"_guardrail_pipelines": ["from-metadata"]}, "litellm_metadata": {"_guardrail_pipelines": ["from-litellm"]}}),
    Some(json!({"_guardrail_pipelines": ["from-metadata"]}))
)]
#[case::metadata_not_a_map(
    json!({"metadata": "text", "litellm_metadata": {"_pipeline_managed_guardrails": []}}),
    Some(json!({"_pipeline_managed_guardrails": []}))
)]
#[case::neither_bucket_qualifies(
    json!({"metadata": {"custom": 1}, "litellm_metadata": {"custom": 2}}),
    None
)]
#[case::null_slot_still_marks_the_bucket(
    json!({"metadata": {"_guardrail_pipelines": null}}),
    Some(json!({"_guardrail_pipelines": null}))
)]
#[case::empty_request(json!({}), None)]
fn policy_state_metadata_picks_the_first_qualifying_bucket(
    #[case] data: JsonValue,
    #[case] expected: Option<JsonValue>,
) {
    let request = data.as_object().unwrap().clone();
    let found = policy_state_metadata(&request).map(|bucket| JsonValue::Object(bucket.clone()));
    assert_eq!(found, expected);
}

#[test]
fn policy_pipelines_reads_stored_pairs_in_order() {
    let request = data_with(&[
        ("policy-a", pipeline(PipelineMode::PostCall, &["pii"])),
        ("policy-b", pipeline(PipelineMode::PreCall, &["block"])),
    ]);
    let pipelines = policy_pipelines(&request);
    let names: Vec<&str> = pipelines.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["policy-a", "policy-b"]);
    assert_eq!(pipelines[1].1, pipeline(PipelineMode::PreCall, &["block"]));
}

#[test]
fn policy_pipelines_ignores_malformed_entries() {
    let bucket = json!({"_guardrail_pipelines": [
        ["policy-a", {"mode": "post_call", "steps": [{"guardrail": "pii"}]}],
        "not-a-pair",
        ["policy-b"],
        ["policy-c", {"mode": "during_call", "steps": [{"guardrail": "pii"}]}],
        ["policy-d", {"mode": "pre_call", "steps": []}],
        ["policy-e", {"mode": "pre_call", "steps": [{"guardrail": "block"}]}],
    ]});
    let request = object(vec![("metadata", bucket)]);
    let (pipelines, log) = capture_warnings(|| policy_pipelines(&request));
    let names: Vec<&str> = pipelines.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["policy-a", "policy-e"]);
    assert!(log.contains("WARN"), "{log}");
}

#[rstest]
#[case::no_bucket(json!({"model": "gpt-4o"}))]
#[case::no_slot(json!({"metadata": {"custom": 1}}))]
#[case::slot_not_an_array(json!({"metadata": {"_guardrail_pipelines": "pii"}}))]
fn policy_pipelines_is_empty_without_stored_pairs(#[case] data: JsonValue) {
    let request = data.as_object().unwrap().clone();
    assert!(policy_pipelines(&request).is_empty());
}

#[test]
fn step_guardrail_names_unions_steps() {
    let pipelines = vec![
        (
            "policy-a".to_owned(),
            pipeline(PipelineMode::PostCall, &["pii", "block"]),
        ),
        (
            "policy-b".to_owned(),
            pipeline(PipelineMode::PreCall, &["pii", "moderation"]),
        ),
    ];
    let expected = BTreeSet::from([
        "block".to_owned(),
        "moderation".to_owned(),
        "pii".to_owned(),
    ]);
    assert_eq!(step_guardrail_names(&pipelines), expected);
}

#[test]
fn pipeline_managed_names_follows_the_mode() {
    let request = data_with(&[
        ("policy-a", pipeline(PipelineMode::PostCall, &["pii"])),
        ("policy-b", pipeline(PipelineMode::PreCall, &["block"])),
    ]);
    assert_eq!(
        pipeline_managed_names(&request, PipelineMode::PreCall),
        BTreeSet::from(["block".to_owned()])
    );
    assert_eq!(
        pipeline_managed_names(&request, PipelineMode::PostCall),
        BTreeSet::from(["pii".to_owned()])
    );
    let post_call = post_call_pipelines(&request);
    let names: Vec<&str> = post_call.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["policy-a"]);
}

#[test]
fn merge_pipeline_metadata_writes_copies_writes_but_not_guardrails() {
    let mut request = object(vec![(
        "metadata",
        json!({"keep": 1, "guardrails": ["pii"], "applied_guardrails": ["old"]}),
    )]);
    let modified = object(vec![(
        "metadata",
        json!({"applied_guardrails": ["pii"], "guardrails": ["pii"], "logged": true}),
    )]);
    merge_pipeline_metadata_writes(&mut request, &modified);
    assert_eq!(
        request.get("metadata"),
        Some(&json!({
            "keep": 1,
            "guardrails": ["pii"],
            "applied_guardrails": ["pii"],
            "logged": true,
        }))
    );
}

#[test]
fn merge_pipeline_metadata_writes_creates_missing_buckets() {
    let mut request = object(vec![]);
    let modified = object(vec![
        (
            "metadata",
            json!({"applied_guardrails": ["pii"], "guardrails": ["pii"]}),
        ),
        (
            "litellm_metadata",
            json!({"applied_policies": ["policy-a"]}),
        ),
    ]);
    merge_pipeline_metadata_writes(&mut request, &modified);
    assert_eq!(
        request.get("metadata"),
        Some(&json!({"applied_guardrails": ["pii"]}))
    );
    assert_eq!(
        request.get("litellm_metadata"),
        Some(&json!({"applied_policies": ["policy-a"]}))
    );
}

#[test]
fn merge_pipeline_metadata_writes_creates_an_empty_bucket() {
    let mut request = object(vec![]);
    let modified = object(vec![("metadata", json!({"guardrails": ["pii"]}))]);
    merge_pipeline_metadata_writes(&mut request, &modified);
    assert_eq!(request.get("metadata"), Some(&json!({})));
}

#[test]
fn merge_pipeline_metadata_writes_ignores_non_object_writes() {
    let mut request = object(vec![("metadata", json!({"keep": 1}))]);
    let modified = object(vec![
        ("metadata", json!("text")),
        ("litellm_metadata", json!([1, 2])),
    ]);
    merge_pipeline_metadata_writes(&mut request, &modified);
    assert_eq!(request, object(vec![("metadata", json!({"keep": 1}))]));
}

#[test]
fn merge_pipeline_metadata_writes_replaces_a_non_object_bucket() {
    let mut request = object(vec![
        ("metadata", json!("old")),
        ("litellm_metadata", JsonValue::Null),
    ]);
    let modified = object(vec![
        ("metadata", json!({"applied_guardrails": ["pii"]})),
        ("litellm_metadata", json!({"applied_guardrails": ["block"]})),
    ]);
    merge_pipeline_metadata_writes(&mut request, &modified);
    let expected = object(vec![
        ("metadata", json!({"applied_guardrails": ["pii"]})),
        ("litellm_metadata", json!({"applied_guardrails": ["block"]})),
    ]);
    assert_eq!(request, expected);
}

fn response(status: Option<&str>) -> ResponsesApiResponse {
    let mut value = json!({"id": "resp_1", "created_at": 0, "output": []});
    if let Some(status) = status {
        value["status"] = json!(status);
    }
    serde_json::from_value(value).unwrap()
}

#[rstest]
#[case::queued(Some("queued"), true)]
#[case::in_progress(Some("in_progress"), true)]
#[case::completed(Some("completed"), false)]
#[case::failed(Some("failed"), false)]
#[case::absent(None, false)]
fn pending_background_response_follows_the_status(
    #[case] status: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(is_pending_background_response(&response(status)), expected);
}
