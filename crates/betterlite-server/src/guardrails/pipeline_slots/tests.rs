#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::read_to_string;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::settings::JsonValue;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_policy::pipeline::{GuardrailPipeline, PipelineMode};
use rstest::rstest;
use serde_json::json;
use tracing::Level;
use tracing::subscriber::with_default;

use super::deferral::{DeferralContext, defer_post_call_pipelines};
use super::model_guardrails::{
    Deployment, DeploymentGuardrails, merge_guardrails_with_existing, merge_model_level_guardrails,
};
use super::slots::{
    is_pending_background_response, merge_pipeline_metadata_writes, pipeline_managed_names,
    policy_pipelines, policy_state_metadata, post_call_pipelines, step_guardrail_names,
};
use super::streaming::{
    PipelineCallbacks, StreamingSupport, pipeline_is_streamable, step_supports_streaming,
    stream_gated_guardrail_names, streamable_post_call_pipelines, unsupported_stream_guardrails,
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

struct FakeTranslation<const ASSEMBLES: bool>;

impl<const ASSEMBLES: bool> BaseTranslation for FakeTranslation<ASSEMBLES> {
    type Guardrail = ();
    type LogTarget = ();
    type Failure = GuardrailFailure;

    const ASSEMBLES_STREAMED_RESPONSE: bool = ASSEMBLES;

    fn process_input_messages(
        &self,
        _data: &mut BTreeMap<String, GuardrailValue>,
        _guardrail: &Self::Guardrail,
        _logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        ready(Ok(()))
    }

    fn process_output_response(
        &self,
        response: GuardrailValue,
        _guardrail: &Self::Guardrail,
        _call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Failure>> + Send {
        ready(Ok(response))
    }
}

struct FakeCallbacks(Vec<(&'static str, StreamingSupport)>);

impl PipelineCallbacks for FakeCallbacks {
    fn find_guardrail_callback(&self, guardrail_name: &str) -> Option<StreamingSupport> {
        self.0
            .iter()
            .find(|(name, _)| *name == guardrail_name)
            .map(|(_, support)| *support)
    }
}

fn callbacks(entries: &[(&'static str, bool, bool)]) -> FakeCallbacks {
    let supports = entries
        .iter()
        .map(|(name, unified, streaming)| {
            (
                *name,
                StreamingSupport {
                    unified_execution: *unified,
                    streaming_execution: *streaming,
                },
            )
        })
        .collect();
    FakeCallbacks(supports)
}

#[test]
fn step_supports_streaming_rejects_an_unknown_guardrail() {
    let callbacks = callbacks(&[]);
    let translation = FakeTranslation::<true>;
    assert!(!step_supports_streaming(
        "pii",
        Some(&translation),
        &callbacks
    ));
}

#[test]
fn step_supports_streaming_accepts_unified_execution() {
    let callbacks = callbacks(&[("pii", true, false)]);
    let translation = FakeTranslation::<false>;
    assert!(step_supports_streaming::<FakeTranslation<false>>(
        "pii", None, &callbacks
    ));
    assert!(step_supports_streaming(
        "pii",
        Some(&translation),
        &callbacks
    ));
}

#[test]
fn step_supports_streaming_needs_a_translation_that_assembles_the_response() {
    let callbacks = callbacks(&[("pii", false, true)]);
    assert!(!step_supports_streaming::<FakeTranslation<false>>(
        "pii", None, &callbacks
    ));
    let plain = FakeTranslation::<false>;
    assert!(!step_supports_streaming("pii", Some(&plain), &callbacks));
    let assembling = FakeTranslation::<true>;
    assert!(step_supports_streaming(
        "pii",
        Some(&assembling),
        &callbacks
    ));
}

#[test]
fn step_supports_streaming_needs_streaming_execution() {
    let callbacks = callbacks(&[("pii", false, false)]);
    let assembling = FakeTranslation::<true>;
    assert!(!step_supports_streaming(
        "pii",
        Some(&assembling),
        &callbacks
    ));
}

#[test]
fn unsupported_stream_guardrails_keeps_first_seen_order_without_repeats() {
    let callbacks = callbacks(&[("pii", false, false), ("block", true, false)]);
    let assembling = FakeTranslation::<true>;
    let steps = pipeline(
        PipelineMode::PostCall,
        &["pii", "block", "pii", "moderation"],
    );
    assert_eq!(
        unsupported_stream_guardrails(&steps, Some(&assembling), &callbacks),
        ["pii", "moderation"]
    );
}

#[test]
fn unsupported_stream_guardrails_is_empty_when_every_step_runs() {
    let callbacks = callbacks(&[("pii", true, false), ("block", true, false)]);
    let steps = pipeline(PipelineMode::PostCall, &["pii", "block"]);
    assert!(
        unsupported_stream_guardrails::<FakeTranslation<false>>(&steps, None, &callbacks)
            .is_empty()
    );
}

#[test]
fn pipeline_is_streamable_warns_about_dropped_guardrails() {
    let callbacks = callbacks(&[]);
    let steps = pipeline(PipelineMode::PostCall, &["pii", "block"]);
    let (streamable, log) = capture_warnings(|| {
        pipeline_is_streamable::<FakeTranslation<false>>("policy-a", &steps, None, &callbacks)
    });
    assert!(!streamable);
    assert!(log.contains("WARN"), "{log}");
    assert!(log.contains("policy-a"), "{log}");
    assert!(log.contains("pii, block"), "{log}");
}

#[test]
fn pipeline_is_streamable_is_quiet_when_every_step_runs() {
    let callbacks = callbacks(&[("pii", true, false)]);
    let steps = pipeline(PipelineMode::PostCall, &["pii"]);
    let (streamable, log) = capture_warnings(|| {
        pipeline_is_streamable::<FakeTranslation<false>>("policy-a", &steps, None, &callbacks)
    });
    assert!(streamable);
    assert!(!log.contains("WARN"), "{log}");
}

#[test]
fn stream_gated_guardrail_names_needs_a_translation() {
    let callbacks = callbacks(&[("pii", true, false)]);
    let request = data_with(&[("policy-a", pipeline(PipelineMode::PostCall, &["pii"]))]);
    let named = stream_gated_guardrail_names::<FakeTranslation<true>>(&request, None, &callbacks);
    assert!(named.is_empty());
}

#[test]
fn stream_gated_guardrail_names_keeps_only_streamable_post_call_pipelines() {
    let callbacks = callbacks(&[("pii", true, false), ("block", false, true)]);
    let assembling = FakeTranslation::<true>;
    let request = data_with(&[
        ("policy-a", pipeline(PipelineMode::PostCall, &["pii"])),
        (
            "policy-b",
            pipeline(PipelineMode::PostCall, &["block", "moderation"]),
        ),
        ("policy-c", pipeline(PipelineMode::PreCall, &["moderation"])),
    ]);
    let named = stream_gated_guardrail_names(&request, Some(&assembling), &callbacks);
    assert_eq!(named, BTreeSet::from(["pii".to_owned()]));
}

#[test]
fn stream_gated_guardrail_names_requires_an_assembling_translation() {
    let callbacks = callbacks(&[("block", false, true)]);
    let plain = FakeTranslation::<false>;
    let request = data_with(&[("policy-b", pipeline(PipelineMode::PostCall, &["block"]))]);
    let named = stream_gated_guardrail_names(&request, Some(&plain), &callbacks);
    assert!(named.is_empty());
}

#[test]
fn streamable_post_call_pipelines_stays_quiet_without_post_call_pipelines() {
    let callbacks = callbacks(&[]);
    let request = data_with(&[("policy-a", pipeline(PipelineMode::PreCall, &["pii"]))]);
    let (pipelines, log) = capture_warnings(|| {
        streamable_post_call_pipelines::<FakeTranslation<true>>(
            &request,
            None,
            Some("/v1/responses"),
            &callbacks,
        )
    });
    assert!(pipelines.is_empty());
    assert!(!log.contains("WARN"), "{log}");
}

#[test]
fn streamable_post_call_pipelines_warns_with_the_route_without_a_translation() {
    let callbacks = callbacks(&[]);
    let request = data_with(&[
        ("policy-a", pipeline(PipelineMode::PostCall, &["pii"])),
        ("policy-b", pipeline(PipelineMode::PostCall, &["block"])),
    ]);
    let (pipelines, log) = capture_warnings(|| {
        streamable_post_call_pipelines::<FakeTranslation<true>>(
            &request,
            None,
            Some("/v1/responses"),
            &callbacks,
        )
    });
    assert!(pipelines.is_empty());
    assert!(log.contains("WARN"), "{log}");
    assert!(log.contains("/v1/responses"), "{log}");
    assert!(log.contains("policy-a, policy-b"), "{log}");
}

#[test]
fn streamable_post_call_pipelines_drops_an_unstreamable_pipeline() {
    let callbacks = callbacks(&[("pii", true, false)]);
    let assembling = FakeTranslation::<true>;
    let request = data_with(&[
        ("policy-a", pipeline(PipelineMode::PostCall, &["pii"])),
        (
            "policy-b",
            pipeline(PipelineMode::PostCall, &["pii", "block"]),
        ),
    ]);
    let (pipelines, log) = capture_warnings(|| {
        streamable_post_call_pipelines(
            &request,
            Some(&assembling),
            Some("/v1/responses"),
            &callbacks,
        )
    });
    let names: Vec<&str> = pipelines.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["policy-a"]);
    assert!(log.contains("WARN"), "{log}");
    assert!(log.contains("policy-b"), "{log}");
    assert!(log.contains("block"), "{log}");
}

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

struct FakeDeployments {
    by_id: BTreeMap<String, Deployment>,
    by_name: BTreeMap<(String, Option<String>), Vec<Deployment>>,
}

impl DeploymentGuardrails for FakeDeployments {
    fn get_deployment(&self, model_id: &str) -> Option<Deployment> {
        self.by_id.get(model_id).cloned()
    }

    fn get_model_list(&self, model_name: &str, team_id: Option<&str>) -> Vec<Deployment> {
        let key = (model_name.to_owned(), team_id.map(str::to_owned));
        self.by_name.get(&key).cloned().unwrap_or_default()
    }
}

fn deployment(guardrails: JsonValue) -> Deployment {
    Deployment {
        guardrails: Some(guardrails),
    }
}

fn deployments_by_id(model_id: &str, guardrails: JsonValue) -> BTreeMap<String, Deployment> {
    BTreeMap::from([(model_id.to_owned(), deployment(guardrails))])
}

fn deployments_by_name(
    model_name: &str,
    team_id: Option<&str>,
    guardrails: Vec<JsonValue>,
) -> BTreeMap<(String, Option<String>), Vec<Deployment>> {
    let entries = guardrails.into_iter().map(deployment).collect();
    BTreeMap::from([((model_name.to_owned(), team_id.map(str::to_owned)), entries)])
}

#[test]
fn model_guardrails_without_a_router_return_the_data_unchanged() {
    let request = object(vec![("metadata", json!("broken"))]);
    let merged = merge_model_level_guardrails(&request, None, true, None).unwrap();
    assert_eq!(merged, request);
}

#[test]
fn model_guardrails_from_a_trusted_deployment_id_merge_into_a_copy() {
    let request = object(vec![(
        "metadata",
        json!({"model_info": {"id": "dep-1"}, "guardrails": ["pii"]}),
    )]);
    let lookup = FakeDeployments {
        by_id: deployments_by_id("dep-1", json!(["block"])),
        by_name: BTreeMap::new(),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json!({
            "model_info": {"id": "dep-1"},
            "guardrails": ["pii", "block"],
        }))
    );
    assert_eq!(
        request.get("metadata"),
        Some(&json!({"model_info": {"id": "dep-1"}, "guardrails": ["pii"]}))
    );
}

#[test]
fn model_guardrails_wrap_a_bare_string_from_the_deployment() {
    let request = object(vec![("metadata", json!({"model_info": {"id": "dep-1"}}))]);
    let lookup = FakeDeployments {
        by_id: deployments_by_id("dep-1", json!("block")),
        by_name: BTreeMap::new(),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json!({"model_info": {"id": "dep-1"}, "guardrails": ["block"]}))
    );
}

#[test]
fn model_guardrails_without_a_deployment_setting_leave_the_data() {
    let request = object(vec![("metadata", json!({"model_info": {"id": "dep-1"}}))]);
    let lookup = FakeDeployments {
        by_id: BTreeMap::from([("dep-1".to_owned(), Deployment { guardrails: None })]),
        by_name: BTreeMap::new(),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(merged, request);
}

#[test]
fn model_guardrails_with_an_empty_list_still_normalize_the_metadata() {
    let request = object(vec![(
        "metadata",
        json!({"model_info": {"id": "dep-1"}, "guardrails": "pii"}),
    )]);
    let lookup = FakeDeployments {
        by_id: deployments_by_id("dep-1", json!([])),
        by_name: BTreeMap::new(),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json!({"model_info": {"id": "dep-1"}, "guardrails": ["pii"]}))
    );
}

#[test]
fn model_guardrails_with_an_unknown_id_leave_the_data() {
    let request = object(vec![
        ("model", json!("alias")),
        ("metadata", json!({"model_info": {"id": "missing"}})),
    ]);
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name: deployments_by_name("alias", None, vec![json!(["pii"])]),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(merged, request);
}

#[test]
fn model_guardrails_treat_a_non_string_id_as_an_unknown_deployment() {
    let request = object(vec![
        ("model", json!("alias")),
        ("metadata", json!({"model_info": {"id": 7}})),
    ]);
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name: deployments_by_name("alias", None, vec![json!(["pii"])]),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(merged, request);
}

#[test]
fn model_guardrails_ignore_the_id_when_client_model_info_is_untrusted() {
    let request = object(vec![
        ("model", json!("alias")),
        ("metadata", json!({"model_info": {"id": "dep-1"}})),
    ]);
    let lookup = FakeDeployments {
        by_id: deployments_by_id("dep-1", json!(["block"])),
        by_name: deployments_by_name("alias", None, vec![json!(["pii"])]),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), false, None).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json!({"model_info": {"id": "dep-1"}, "guardrails": ["pii"]}))
    );
}

#[test]
fn model_guardrails_union_the_alias_group_for_the_team() {
    let request = object(vec![
        ("model", json!("ignored")),
        ("metadata", json!({"user_api_key_team_id": "team-a"})),
    ]);
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name: deployments_by_name(
            "alias",
            Some("team-a"),
            vec![json!(["pii"]), json!(["block", "pii"])],
        ),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, Some("alias")).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json!({
            "user_api_key_team_id": "team-a",
            "guardrails": ["pii", "block"],
        }))
    );
}

#[test]
fn model_guardrails_read_the_team_from_litellm_metadata() {
    let request = object(vec![
        ("model", json!("alias")),
        (
            "litellm_metadata",
            json!({"user_api_key_team_id": "team-b"}),
        ),
    ]);
    let distinct = deployments_by_name("alias", Some("team-b"), vec![json!(["pii"])]);
    let other = deployments_by_name("alias", None, vec![json!(["block"])]);
    let by_name = distinct.into_iter().chain(other).collect();
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name,
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json!({"guardrails": ["pii"]}))
    );
}

#[test]
fn model_guardrails_treat_a_non_string_team_id_as_unset() {
    let request = object(vec![
        ("model", json!("alias")),
        ("metadata", json!({"user_api_key_team_id": 7})),
    ]);
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name: deployments_by_name("alias", None, vec![json!(["pii"])]),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json!({"user_api_key_team_id": 7, "guardrails": ["pii"]}))
    );
}

#[rstest]
#[case::missing_model(object(vec![]))]
#[case::non_string_model(object(vec![("model", json!(7))]))]
#[case::empty_model(object(vec![("model", json!(""))]))]
fn model_guardrails_without_an_alias_leave_the_data(#[case] request: JsonMap) {
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name: deployments_by_name("", None, vec![json!(["pii"])]),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(merged, request);
}

#[rstest]
#[case::metadata(object(vec![("metadata", json!([1, 2]))]), "metadata")]
#[case::litellm_metadata(object(vec![("litellm_metadata", json!("text"))]), "litellm_metadata")]
#[case::model_info(object(vec![("metadata", json!({"model_info": 5}))]), "metadata.model_info")]
fn model_guardrails_name_the_misplaced_object(#[case] request: JsonMap, #[case] field: &str) {
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name: BTreeMap::new(),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let error = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap_err();
    assert_eq!(error.to_string(), format!("'{field}' must be an object"));
}

#[test]
fn guardrail_merge_creates_metadata_and_unions_names() {
    let request = object(vec![
        ("model", json!("gpt-4o")),
        ("metadata", json!({"team": "acme"})),
    ]);
    let merged = merge_guardrails_with_existing(&request, &json!(["pii", "block"])).unwrap();
    assert_eq!(
        merged,
        object(vec![
            ("model", json!("gpt-4o")),
            (
                "metadata",
                json!({"team": "acme", "guardrails": ["pii", "block"]}),
            ),
        ])
    );
}

#[test]
fn guardrail_merge_wraps_scalars_on_both_sides() {
    let request = object(vec![("metadata", json!({"guardrails": "pii"}))]);
    let merged = merge_guardrails_with_existing(&request, &json!("block")).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json!({"guardrails": ["pii", "block"]}))
    );
}

#[test]
fn guardrail_merge_deduplicates_names() {
    let request = object(vec![(
        "metadata",
        json!({"guardrails": ["pii", "block"], "team": "acme"}),
    )]);
    let merged =
        merge_guardrails_with_existing(&request, &json!(["block", "pii", "moderation"])).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json!({
            "guardrails": ["pii", "block", "moderation"],
            "team": "acme",
        }))
    );
}

#[test]
fn guardrail_merge_keeps_a_nested_entry_once() {
    let request = object(vec![("metadata", json!({"guardrails": [{"name": "pii"}]}))]);
    let merged =
        merge_guardrails_with_existing(&request, &json!([{"name": "pii"}, "block"])).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json!({"guardrails": [{"name": "pii"}, "block"]}))
    );
}

#[test]
fn guardrail_merge_treats_a_null_model_level_list_as_empty() {
    let request = object(vec![]);
    let merged = merge_guardrails_with_existing(&request, &JsonValue::Null).unwrap();
    assert_eq!(
        merged,
        object(vec![("metadata", json!({"guardrails": []}))])
    );
}

#[test]
fn guardrail_merge_treats_a_null_metadata_as_absent() {
    let request = object(vec![("metadata", JsonValue::Null)]);
    let merged = merge_guardrails_with_existing(&request, &json!(["pii"])).unwrap();
    assert_eq!(
        merged,
        object(vec![("metadata", json!({"guardrails": ["pii"]}))])
    );
}

#[test]
fn guardrail_merge_never_changes_the_given_request() {
    let request = object(vec![("metadata", json!({"guardrails": ["pii"]}))]);
    let merged = merge_guardrails_with_existing(&request, &json!(["block"])).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json!({"guardrails": ["pii", "block"]}))
    );
    assert_eq!(
        request.get("metadata"),
        Some(&json!({"guardrails": ["pii"]}))
    );
}

#[test]
fn guardrail_merge_rejects_a_non_object_metadata() {
    let request = object(vec![("metadata", json!(7))]);
    let error = merge_guardrails_with_existing(&request, &json!(["pii"])).unwrap_err();
    assert_eq!(error.to_string(), "'metadata' must be an object");
}
