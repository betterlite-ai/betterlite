#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_policy::pipeline::{GuardrailPipeline, PipelineMode};
use serde_json::json;

use super::super::deferral::{DeferralContext, defer_post_call_pipelines};
use super::super::slots::post_call_pipelines;
use super::{capture_warnings, object, pipeline, pipeline_pairs};

struct FakeContext {
    resolved: BTreeMap<String, Vec<String>>,
    standalone: BTreeSet<String>,
}

impl DeferralContext for FakeContext {
    fn resolved_policy_guardrails(&self, policy_name: &str) -> Vec<String> {
        self.resolved.get(policy_name).cloned().unwrap_or_default()
    }

    fn standalone_pre_call_guardrails(&self, _data: &JsonMap) -> BTreeSet<String> {
        self.standalone.clone()
    }
}

fn context(resolved_policies: &[(&str, &[&str])], standalone_names: &[&str]) -> FakeContext {
    let resolved: BTreeMap<String, Vec<String>> = resolved_policies
        .iter()
        .map(|(policy_name, guardrails)| {
            let names = guardrails
                .iter()
                .map(|guardrail| (*guardrail).to_owned())
                .collect();
            ((*policy_name).to_owned(), names)
        })
        .collect();
    let standalone = standalone_names
        .iter()
        .map(|guardrail| (*guardrail).to_owned())
        .collect();
    FakeContext {
        resolved,
        standalone,
    }
}

fn deferral_data(
    policies: &[(&str, GuardrailPipeline)],
    fields: Vec<(&str, JsonValue)>,
) -> JsonMap {
    let mut metadata = object(fields);
    metadata.insert(
        "_guardrail_pipelines".to_owned(),
        JsonValue::Array(pipeline_pairs(policies)),
    );
    object(vec![("metadata", JsonValue::Object(metadata))])
}

fn metadata_of(request: &JsonMap) -> &JsonMap {
    request.get("metadata").unwrap().as_object().unwrap()
}

#[test]
fn deferring_without_post_call_pipelines_leaves_the_request_alone() {
    let mut request = deferral_data(&[], vec![("applied_guardrails", json!(["pii"]))]);
    let expected = request.clone();
    let policies = post_call_pipelines(&request);
    let deferred = context(&[], &[]);
    let (_, log) = capture_warnings(|| {
        defer_post_call_pipelines(&mut request, "resp_1", "queued", &policies, &deferred)
    });
    assert_eq!(request, expected);
    assert!(!log.contains("WARN"), "{log}");
}

#[test]
fn deferring_warns_for_tag_and_body_matches_and_withdraws_claims() {
    let guardrails: &[&str] = &["pii", "block"];
    let mut request = deferral_data(
        &[
            ("policy-tag", pipeline(PipelineMode::PostCall, guardrails)),
            ("policy-body", pipeline(PipelineMode::PostCall, guardrails)),
        ],
        vec![
            (
                "applied_policies",
                json!(["policy-tag", "policy-body", "policy-keep"]),
            ),
            ("applied_guardrails", json!(["pii", "block", "keep"])),
            (
                "policy_sources",
                json!({"policy-tag": "tag:team-a", "policy-keep": "key"}),
            ),
        ],
    );
    let policies = post_call_pipelines(&request);
    let deferred = context(
        &[("policy-tag", guardrails), ("policy-body", guardrails)],
        &[],
    );
    let (_, log) = capture_warnings(|| {
        defer_post_call_pipelines(&mut request, "resp_1", "queued", &policies, &deferred)
    });
    assert!(log.contains("DEBUG"), "{log}");
    assert!(log.contains("resp_1"), "{log}");
    assert!(log.contains("WARN"), "{log}");
    let tag_line = log
        .lines()
        .find(|line| line.contains("request tag"))
        .unwrap();
    assert!(tag_line.contains("policy-tag"), "{log}");
    let body_line = log
        .lines()
        .find(|line| line.contains("request body's policies"))
        .unwrap();
    assert!(body_line.contains("policy-body"), "{log}");
    let metadata = metadata_of(&request);
    assert_eq!(
        metadata.get("applied_policies"),
        Some(&json!(["policy-keep"]))
    );
    assert_eq!(metadata.get("applied_guardrails"), Some(&json!(["keep"])));
    assert_eq!(
        metadata.get("policy_sources"),
        Some(&json!({"policy-keep": "key"}))
    );
}

#[test]
fn deferring_treats_a_non_string_policy_source_as_not_a_tag_match() {
    let mut request = deferral_data(
        &[("policy-a", pipeline(PipelineMode::PostCall, &["pii"]))],
        vec![
            ("applied_policies", json!(["policy-a"])),
            ("policy_sources", json!({"policy-a": ["tag:team-a"]})),
        ],
    );
    let policies = post_call_pipelines(&request);
    let deferred = context(&[("policy-a", &["pii"])], &[]);
    let (_, log) = capture_warnings(|| {
        defer_post_call_pipelines(&mut request, "resp_1", "queued", &policies, &deferred)
    });
    assert!(!log.contains("request tag"), "{log}");
    assert!(!log.contains("request body's policies"), "{log}");
    let metadata = metadata_of(&request);
    assert!(metadata.get("applied_policies").is_none());
    assert!(metadata.get("policy_sources").is_none());
}

#[test]
fn deferring_keeps_a_policy_with_a_guardrail_outside_the_pipeline() {
    let mut request = deferral_data(
        &[("policy-a", pipeline(PipelineMode::PostCall, &["pii"]))],
        vec![
            ("applied_policies", json!(["policy-a"])),
            ("applied_guardrails", json!(["pii"])),
            ("policy_sources", json!({"policy-a": "key"})),
        ],
    );
    let policies = post_call_pipelines(&request);
    let deferred = context(&[("policy-a", &["pii", "outside"])], &[]);
    defer_post_call_pipelines(&mut request, "resp_1", "queued", &policies, &deferred);
    let metadata = metadata_of(&request);
    assert_eq!(metadata.get("applied_policies"), Some(&json!(["policy-a"])));
    assert_eq!(
        metadata.get("policy_sources"),
        Some(&json!({"policy-a": "key"}))
    );
    assert!(metadata.get("applied_guardrails").is_none());
}

#[test]
fn deferring_keeps_step_guardrails_that_run_elsewhere() {
    let mut request = deferral_data(
        &[
            ("policy-a", pipeline(PipelineMode::PostCall, &["pii"])),
            ("policy-pre", pipeline(PipelineMode::PreCall, &["pii"])),
        ],
        vec![
            ("applied_guardrails", json!(["pii", "block"])),
            ("applied_policies", json!("not-a-list")),
        ],
    );
    let policies = post_call_pipelines(&request);
    let deferred = context(&[("policy-a", &["pii"])], &["block"]);
    defer_post_call_pipelines(&mut request, "resp_1", "queued", &policies, &deferred);
    let metadata = metadata_of(&request);
    assert_eq!(
        metadata.get("applied_guardrails"),
        Some(&json!(["pii", "block"]))
    );
    assert_eq!(metadata.get("applied_policies"), Some(&json!("not-a-list")));
}

#[test]
fn deferring_keeps_a_claim_entry_it_cannot_name() {
    let mut request = deferral_data(
        &[("policy-a", pipeline(PipelineMode::PostCall, &["pii"]))],
        vec![("applied_guardrails", json!(["pii", 7]))],
    );
    let policies = post_call_pipelines(&request);
    let deferred = context(&[("policy-a", &["pii"])], &[]);
    defer_post_call_pipelines(&mut request, "resp_1", "queued", &policies, &deferred);
    let metadata = metadata_of(&request);
    assert_eq!(metadata.get("applied_guardrails"), Some(&json!([7])));
}

#[test]
fn deferring_leaves_a_non_object_policy_sources_alone() {
    let mut request = deferral_data(
        &[("policy-a", pipeline(PipelineMode::PostCall, &["pii"]))],
        vec![
            ("applied_policies", json!(["policy-a"])),
            ("policy_sources", json!("attributed elsewhere")),
        ],
    );
    let policies = post_call_pipelines(&request);
    let deferred = context(&[("policy-a", &["pii"])], &[]);
    defer_post_call_pipelines(&mut request, "resp_1", "queued", &policies, &deferred);
    let metadata = metadata_of(&request);
    assert_eq!(
        metadata.get("policy_sources"),
        Some(&json!("attributed elsewhere"))
    );
    assert!(metadata.get("applied_policies").is_none());
}

#[test]
fn deferring_pops_policy_sources_when_every_policy_is_withdrawn() {
    let mut request = deferral_data(
        &[("policy-a", pipeline(PipelineMode::PostCall, &["pii"]))],
        vec![
            ("applied_policies", json!(["policy-a"])),
            ("policy_sources", json!({"policy-a": "body"})),
        ],
    );
    let policies = post_call_pipelines(&request);
    let deferred = context(&[("policy-a", &["pii"])], &[]);
    defer_post_call_pipelines(&mut request, "resp_1", "queued", &policies, &deferred);
    let metadata = metadata_of(&request);
    assert!(metadata.get("policy_sources").is_none());
    assert!(metadata.get("applied_policies").is_none());
}

#[test]
fn deferring_writes_into_the_litellm_metadata_bucket_when_present() {
    let mut request = deferral_data(
        &[("policy-a", pipeline(PipelineMode::PostCall, &["pii"]))],
        vec![("applied_policies", json!(["policy-a"]))],
    );
    request.insert("litellm_metadata".to_owned(), JsonValue::Null);
    let policies = post_call_pipelines(&request);
    let deferred = context(&[("policy-a", &["pii"])], &[]);
    defer_post_call_pipelines(&mut request, "resp_1", "queued", &policies, &deferred);
    assert_eq!(request.get("litellm_metadata"), Some(&json!({})));
    assert_eq!(
        metadata_of(&request).get("applied_policies"),
        Some(&json!(["policy-a"]))
    );
}

#[test]
fn deferring_creates_the_metadata_bucket_when_absent() {
    let mut request = object(vec![]);
    let policies = vec![(
        "policy-a".to_owned(),
        pipeline(PipelineMode::PostCall, &["pii"]),
    )];
    let deferred = context(&[("policy-a", &["pii"])], &[]);
    defer_post_call_pipelines(&mut request, "resp_1", "queued", &policies, &deferred);
    assert_eq!(request.get("metadata"), Some(&json!({})));
}
