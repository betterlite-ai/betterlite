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

fn json_object(entries: Vec<(&str, JsonValue)>) -> JsonValue {
    JsonValue::Object(object(entries))
}

fn pipeline(mode: PipelineMode, guardrails: &[&str]) -> GuardrailPipeline {
    let steps: Vec<JsonValue> = guardrails
        .iter()
        .map(|guardrail| json_object(vec![("guardrail", json!(*guardrail))]))
        .collect();
    let stored = json_object(vec![
        ("mode", json!(mode)),
        ("steps", JsonValue::Array(steps)),
    ]);
    serde_json::from_value(stored).unwrap()
}

fn pipeline_pairs(pipelines: &[(&str, GuardrailPipeline)]) -> Vec<JsonValue> {
    pipelines
        .iter()
        .map(|(policy_name, pipeline)| json!([policy_name, pipeline]))
        .collect()
}

fn pipeline_bucket(pipelines: &[(&str, GuardrailPipeline)]) -> JsonValue {
    json_object(vec![(
        "_guardrail_pipelines",
        JsonValue::Array(pipeline_pairs(pipelines)),
    )])
}

fn data_with(pipelines: &[(&str, GuardrailPipeline)]) -> JsonMap {
    object(vec![("metadata", pipeline_bucket(pipelines))])
}

#[rstest]
#[case::metadata_slot(
    json_object(vec![(
        "metadata",
        json_object(vec![
            ("custom", json!(1_i64)),
            ("_pipeline_managed_guardrails", json!(["pii"])),
        ]),
    )]),
    Some(json_object(vec![
        ("custom", json!(1_i64)),
        ("_pipeline_managed_guardrails", json!(["pii"])),
    ]))
)]
#[case::litellm_metadata_slot(
    json_object(vec![
        ("metadata", json_object(vec![("custom", json!(1_i64))])),
        (
            "litellm_metadata",
            json_object(vec![("_guardrail_pipelines", json!([]))]),
        ),
    ]),
    Some(json_object(vec![("_guardrail_pipelines", json!([]))]))
)]
#[case::metadata_first_when_both_qualify(
    json_object(vec![
        (
            "metadata",
            json_object(vec![("_guardrail_pipelines", json!(["from-metadata"]))]),
        ),
        (
            "litellm_metadata",
            json_object(vec![("_guardrail_pipelines", json!(["from-litellm"]))]),
        ),
    ]),
    Some(json_object(vec![(
        "_guardrail_pipelines",
        json!(["from-metadata"]),
    )]))
)]
#[case::metadata_not_a_map(
    json_object(vec![
        ("metadata", json!("text")),
        (
            "litellm_metadata",
            json_object(vec![("_pipeline_managed_guardrails", json!([]))]),
        ),
    ]),
    Some(json_object(vec![("_pipeline_managed_guardrails", json!([]))]))
)]
#[case::neither_bucket_qualifies(
    json_object(vec![
        ("metadata", json_object(vec![("custom", json!(1_i64))])),
        ("litellm_metadata", json_object(vec![("custom", json!(2_i64))])),
    ]),
    None
)]
#[case::null_slot_still_marks_the_bucket(
    json_object(vec![(
        "metadata",
        json_object(vec![("_guardrail_pipelines", JsonValue::Null)]),
    )]),
    Some(json_object(vec![(
        "_guardrail_pipelines",
        JsonValue::Null,
    )]))
)]
#[case::empty_request(json_object(vec![]), None)]
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
    let pii_steps = JsonValue::Array(vec![json_object(vec![("guardrail", json!("pii"))])]);
    let post_call = json_object(vec![
        ("mode", json!("post_call")),
        ("steps", pii_steps.clone()),
    ]);
    let during_call = json_object(vec![("mode", json!("during_call")), ("steps", pii_steps)]);
    let empty_steps = json_object(vec![("mode", json!("pre_call")), ("steps", json!([]))]);
    let block_steps = json_object(vec![
        ("mode", json!("pre_call")),
        (
            "steps",
            JsonValue::Array(vec![json_object(vec![("guardrail", json!("block"))])]),
        ),
    ]);
    let bucket = json_object(vec![(
        "_guardrail_pipelines",
        JsonValue::Array(vec![
            JsonValue::Array(vec![json!("policy-a"), post_call]),
            json!("not-a-pair"),
            JsonValue::Array(vec![json!("policy-b")]),
            JsonValue::Array(vec![json!("policy-c"), during_call]),
            JsonValue::Array(vec![json!("policy-d"), empty_steps]),
            JsonValue::Array(vec![json!("policy-e"), block_steps]),
        ]),
    )]);
    let request = object(vec![("metadata", bucket)]);
    let (pipelines, log) = capture_warnings(|| policy_pipelines(&request));
    let names: Vec<&str> = pipelines.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["policy-a", "policy-e"]);
    assert!(log.contains("WARN"), "{log}");
}

#[rstest]
#[case::no_bucket(json_object(vec![("model", json!("gpt-4o"))]))]
#[case::no_slot(json_object(vec![(
    "metadata",
    json_object(vec![("custom", json!(1_i64))]),
)]))]
#[case::slot_not_an_array(json_object(vec![(
    "metadata",
    json_object(vec![("_guardrail_pipelines", json!("pii"))]),
)]))]
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
        json_object(vec![
            ("keep", json!(1_i64)),
            ("guardrails", json!(["pii"])),
            ("applied_guardrails", json!(["old"])),
        ]),
    )]);
    let modified = object(vec![(
        "metadata",
        json_object(vec![
            ("applied_guardrails", json!(["pii"])),
            ("guardrails", json!(["pii"])),
            ("logged", json!(true)),
        ]),
    )]);
    merge_pipeline_metadata_writes(&mut request, &modified);
    assert_eq!(
        request.get("metadata"),
        Some(&json_object(vec![
            ("keep", json!(1_i64)),
            ("guardrails", json!(["pii"])),
            ("applied_guardrails", json!(["pii"])),
            ("logged", json!(true)),
        ]))
    );
}

#[test]
fn merge_pipeline_metadata_writes_creates_missing_buckets() {
    let mut request = object(vec![]);
    let modified = object(vec![
        (
            "metadata",
            json_object(vec![
                ("applied_guardrails", json!(["pii"])),
                ("guardrails", json!(["pii"])),
            ]),
        ),
        (
            "litellm_metadata",
            json_object(vec![("applied_policies", json!(["policy-a"]))]),
        ),
    ]);
    merge_pipeline_metadata_writes(&mut request, &modified);
    assert_eq!(
        request.get("metadata"),
        Some(&json_object(vec![("applied_guardrails", json!(["pii"]))]))
    );
    assert_eq!(
        request.get("litellm_metadata"),
        Some(&json_object(vec![(
            "applied_policies",
            json!(["policy-a"])
        )]))
    );
}

#[test]
fn merge_pipeline_metadata_writes_creates_an_empty_bucket() {
    let mut request = object(vec![]);
    let modified = object(vec![(
        "metadata",
        json_object(vec![("guardrails", json!(["pii"]))]),
    )]);
    merge_pipeline_metadata_writes(&mut request, &modified);
    assert_eq!(request.get("metadata"), Some(&json_object(vec![])));
}

#[test]
fn merge_pipeline_metadata_writes_ignores_non_object_writes() {
    let mut request = object(vec![(
        "metadata",
        json_object(vec![("keep", json!(1_i64))]),
    )]);
    let modified = object(vec![
        ("metadata", json!("text")),
        ("litellm_metadata", json!([1_i64, 2_i64])),
    ]);
    merge_pipeline_metadata_writes(&mut request, &modified);
    assert_eq!(
        request,
        object(vec![(
            "metadata",
            json_object(vec![("keep", json!(1_i64))])
        )])
    );
}

#[test]
fn merge_pipeline_metadata_writes_replaces_a_non_object_bucket() {
    let mut request = object(vec![
        ("metadata", json!("old")),
        ("litellm_metadata", JsonValue::Null),
    ]);
    let modified = object(vec![
        (
            "metadata",
            json_object(vec![("applied_guardrails", json!(["pii"]))]),
        ),
        (
            "litellm_metadata",
            json_object(vec![("applied_guardrails", json!(["block"]))]),
        ),
    ]);
    merge_pipeline_metadata_writes(&mut request, &modified);
    let expected = object(vec![
        (
            "metadata",
            json_object(vec![("applied_guardrails", json!(["pii"]))]),
        ),
        (
            "litellm_metadata",
            json_object(vec![("applied_guardrails", json!(["block"]))]),
        ),
    ]);
    assert_eq!(request, expected);
}

fn response(status: Option<&str>) -> ResponsesApiResponse {
    let mut value = json_object(vec![
        ("id", json!("resp_1")),
        ("created_at", json!(0_i64)),
        ("output", JsonValue::Array(vec![])),
    ]);
    if let Some(label) = status {
        value["status"] = json!(label);
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
