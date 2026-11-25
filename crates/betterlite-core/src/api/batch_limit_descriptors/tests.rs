#![cfg(test)]

mod support;

use std::collections::BTreeMap;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use betterlite_auth::resolved_key::credential::safe_hash_api_key;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::BatchLimitDescriptors;
use super::BatchLimitError;
use super::answer::BatchLimitAnswer;
use super::descriptors::{
    BatchDescriptorIncrements, BatchLimitDescriptor, TPD_SUFFIX, TPD_WINDOW_SECONDS,
};
use crate::api::file_routing_ids::RoutingIdType;
use crate::api::file_routing_ids::routing::encode_file_id_with_model;
use support::{
    FakeCredentials, FakeLimiter, ITPM, OTPM, descriptor, limiter, router, service, settings,
};

const WARNING: &str = "general_settings.skip_batch_input_file_rate_limiting_for_models is not supported and has no effect. Use skip_batch_input_file_rate_limiting_for_providers or disable_batch_input_file_rate_limiting instead.";
const UNIFIED_ID: &str =
    "litellm_proxy:application/octet-stream;unified_id,uid;target_model_names,model-a,model-b";
const TPD_PRINCIPAL: &str = r#"{"api_key": "key-batch", "tpd_limit": 1000, "team_id": "team-1", "team_tpd_limit": 0, "end_user_id": "user-1", "end_user_tpd_limit": 3000}"#;
const PROJECT_LIMITS_CALL: &str = r#"[{"function": "project_has_io_limits"}]"#;
const ACCESS_CHECK_CALL: &str = r#"[{"function": "key_requires_access_check"}]"#;

fn run_calls(
    service: &BatchLimitDescriptors<FakeLimiter, FakeCredentials>,
    principal_fields: &str,
    calls: &str,
) -> Vec<BatchLimitAnswer> {
    service
        .run(&support::input(principal_fields, calls))
        .expect("the calls answer")
        .answers
}

fn empty_limiter() -> FakeLimiter {
    limiter(Vec::new(), Vec::new(), 1, 1024, false)
}

fn single(answers: Vec<BatchLimitAnswer>) -> BatchLimitAnswer {
    let mut remaining = answers.into_iter();
    let answer = remaining.next().expect("one answer");
    assert!(remaining.next().is_none(), "exactly one answer");
    answer
}

fn model_answer(answer: BatchLimitAnswer) -> Option<String> {
    let BatchLimitAnswer::FileBoundModel { model } = answer else {
        panic!("unexpected answer {answer:?}");
    };
    model
}

fn routing_answer(answer: BatchLimitAnswer) -> Option<String> {
    let BatchLimitAnswer::BatchRoutingModel { model } = answer else {
        panic!("unexpected answer {answer:?}");
    };
    model
}

fn skip_answer(answer: BatchLimitAnswer) -> (bool, Option<Vec<BatchLimitDescriptor>>) {
    let BatchLimitAnswer::ShouldSkip { skip, descriptors } = answer else {
        panic!("unexpected answer {answer:?}");
    };
    (skip, descriptors)
}

fn project_io_flag(answer: &BatchLimitAnswer) -> bool {
    let BatchLimitAnswer::ProjectHasIoLimits { has_limits } = answer else {
        panic!("unexpected answer {answer:?}");
    };
    *has_limits
}

fn access_check_flag(answer: &BatchLimitAnswer) -> bool {
    let BatchLimitAnswer::KeyRequiresAccessCheck { required } = answer else {
        panic!("unexpected answer {answer:?}");
    };
    *required
}

fn applicable_flag(answer: &BatchLimitAnswer) -> bool {
    let BatchLimitAnswer::HasApplicableLimits { applicable } = answer else {
        panic!("unexpected answer {answer:?}");
    };
    *applicable
}

fn descriptors_answer(answer: BatchLimitAnswer) -> Vec<BatchLimitDescriptor> {
    let BatchLimitAnswer::CreateRateLimitDescriptors { descriptors } = answer else {
        panic!("unexpected answer {answer:?}");
    };
    descriptors
}

fn project_io_answer(
    answer: BatchLimitAnswer,
) -> (Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>) {
    let BatchLimitAnswer::CreateProjectIoDescriptors {
        descriptors,
        increments,
    } = answer
    else {
        panic!("unexpected answer {answer:?}");
    };
    (descriptors, increments)
}

fn tokens_answer(answer: &BatchLimitAnswer) -> i64 {
    let BatchLimitAnswer::EstimateOutputTokens { tokens } = answer else {
        panic!("unexpected answer {answer:?}");
    };
    *tokens
}

fn fetch_params_answer(answer: BatchLimitAnswer) -> (String, JsonObject) {
    let BatchLimitAnswer::ResolveFetchParams { file_id, params } = answer else {
        panic!("unexpected answer {answer:?}");
    };
    (file_id, params)
}

fn provider_answer(answer: BatchLimitAnswer) -> Option<String> {
    let BatchLimitAnswer::ResolveBatchProvider { provider } = answer else {
        panic!("unexpected answer {answer:?}");
    };
    provider
}

#[rstest]
#[case::embedded_file_id("embedded", Some("bound-model"), Some("bound-model"))]
#[case::unified_file_id("unified", Some("model-a"), Some("model-a"))]
#[case::plain_file_id("plain", None, Some("top-model"))]
fn file_bound_model_precedence(
    #[case] kind: &str,
    #[case] bound: Option<&str>,
    #[case] routed: Option<&str>,
) {
    let file_id = match kind {
        "embedded" => {
            encode_file_id_with_model("file-original", "bound-model", RoutingIdType::File)
        }
        "unified" => URL_SAFE_NO_PAD.encode(UNIFIED_ID),
        other => {
            assert_eq!(other, "plain", "a known fixture");
            "file-original".to_owned()
        }
    };
    let data = format!(r#"{{"input_file_id": "{file_id}", "model": "top-model"}}"#);
    let calls = format!(
        r#"[{{"function": "file_bound_model", "data": {data}}}, {{"function": "batch_routing_model", "data": {data}}}]"#
    );
    let service = service(settings(None, None, None), empty_limiter(), None);
    let answers = run_calls(&service, "{}", &calls);
    assert_eq!(model_answer(answers[0].clone()).as_deref(), bound);
    assert_eq!(routing_answer(answers[1].clone()).as_deref(), routed);
}

#[test]
fn file_bound_model_without_input_file_id_or_model() {
    let service = service(settings(None, None, None), empty_limiter(), None);
    let answer = single(run_calls(
        &service,
        "{}",
        r#"[{"function": "batch_routing_model", "data": {}}]"#,
    ));
    assert_eq!(routing_answer(answer), None);
}

#[test]
fn blank_strings_are_absent() {
    let service = service(settings(None, None, None), empty_limiter(), None);
    let answer = single(run_calls(
        &service,
        "{}",
        r#"[{"function": "batch_routing_model", "data": {"input_file_id": "", "model": ""}}]"#,
    ));
    assert_eq!(routing_answer(answer), None);
}

#[test]
fn project_io_descriptors() {
    let principal = r#"{"project_id": "proj-1", "project_metadata": {"model_itpm_limit": {"gpt": 100}, "model_otpm_limit": {"gpt": 50}}}"#;
    let project = vec![
        descriptor(ITPM, "quota", None, Some(100)),
        descriptor(OTPM, "quota", None, Some(50)),
    ];
    let service = service(
        settings(None, None, None),
        limiter(Vec::new(), project, 1, 1024, false),
        None,
    );
    let calls = r#"[{"function": "create_project_io_descriptors", "per_model_usage": {"gpt": {"total_tokens": 10, "output_tokens": 3}, "claude": {"total_tokens": 7}}}, {"function": "project_has_io_limits"}]"#;
    let answers = run_calls(&service, principal, calls);
    let (descriptors, increments) = project_io_answer(answers[0].clone());
    let values: Vec<(&str, &str)> = descriptors
        .iter()
        .map(|entry| (entry.key.as_str(), entry.value.as_str()))
        .collect();
    assert_eq!(
        values,
        vec![
            (ITPM, "gpt:quota"),
            (OTPM, "gpt:quota"),
            (ITPM, "claude:quota"),
            (OTPM, "claude:quota"),
        ]
    );
    let charges: Vec<(i64, i64)> = increments
        .iter()
        .map(|entry| (entry.requests, entry.tokens))
        .collect();
    assert_eq!(charges, vec![(0, 10), (0, 3), (0, 7), (0, 0)]);
    assert!(project_io_flag(&answers[1]));
}

#[rstest]
#[case::project_metadata(
    r#"{"project_id": "proj-1", "project_metadata": {"model_otpm_limit": {"gpt": 5}}}"#,
    true
)]
#[case::no_project_id(r#"{"project_metadata": {"model_otpm_limit": {"gpt": 5}}}"#, false)]
#[case::no_metadata(r#"{"project_id": "proj-1"}"#, false)]
#[case::empty_metadata_maps(
    r#"{"project_id": "proj-1", "project_metadata": {"model_itpm_limit": {}, "model_otpm_limit": {}}}"#,
    false
)]
#[case::one_map_configured(
    r#"{"project_id": "proj-1", "project_metadata": {"model_itpm_limit": {"gpt": 5}, "model_otpm_limit": {}}}"#,
    true
)]
fn project_io_has_limits(#[case] principal: &str, #[case] expected: bool) {
    let service = service(settings(None, None, None), empty_limiter(), None);
    let answer = single(run_calls(&service, principal, PROJECT_LIMITS_CALL));
    assert_eq!(project_io_flag(&answer), expected);
}

#[rstest]
#[case::embeddings(r#"{"url": "/v1/embeddings", "body": {"max_tokens": 99}}"#, 1, 0)]
#[case::max_tokens(r#"{"body": {"max_tokens": 100}}"#, 1, 100)]
#[case::max_completion_tokens(r#"{"body": {"max_completion_tokens": 40}}"#, 1, 40)]
#[case::max_output_tokens(r#"{"body": {"max_output_tokens": 25}}"#, 1, 25)]
#[case::first_non_null_wins(
    r#"{"body": {"max_tokens": null, "max_completion_tokens": 40, "max_output_tokens": 25}}"#,
    1,
    40
)]
#[case::floor_when_absent(r#"{"body": {}}"#, 1, 1024)]
#[case::floor_when_body_missing("{}", 1, 1024)]
#[case::negative_cap(r#"{"body": {"max_tokens": -5}}"#, 1, 0)]
#[case::float_cap(r#"{"body": {"max_tokens": 12.5}}"#, 1, 12)]
#[case::negative_float_cap(r#"{"body": {"max_tokens": -0.5}}"#, 1, 0)]
#[case::bool_cap(r#"{"body": {"max_tokens": true}}"#, 1, 1)]
#[case::false_cap(r#"{"body": {"max_tokens": false}}"#, 1, 0)]
#[case::text_cap(r#"{"body": {"max_tokens": "12"}}"#, 1, 12)]
#[case::text_cap_with_spaces(r#"{"body": {"max_tokens": " 12 "}}"#, 1, 12)]
#[case::unparsable_cap(r#"{"body": {"max_tokens": "abc"}}"#, 1, 1024)]
#[case::candidates_multiply(r#"{"body": {"max_tokens": 10}}"#, 3, 30)]
#[case::floor_candidates_multiply(r#"{"body": {}}"#, 3, 3072)]
#[case::huge_cap_saturates(r#"{"body": {"max_tokens": 9223372036854775807}}"#, 3, i64::MAX)]
#[case::cap_beyond_i64_falls_back(r#"{"body": {"max_tokens": 9223372036854775808}}"#, 1, 1024)]
fn max_output_estimate(#[case] entry: &str, #[case] candidates: i64, #[case] expected: i64) {
    let service = service(
        settings(None, None, None),
        limiter(Vec::new(), Vec::new(), candidates, 1024, false),
        None,
    );
    let calls = format!(
        r#"[{{"function": "estimate_output_tokens", "entry": {entry}, "min_configured_otpm_limit": null}}]"#
    );
    let answer = single(run_calls(&service, "{}", &calls));
    assert_eq!(tokens_answer(&answer), expected);
}

#[test]
fn unsupported_model_skip_warning() {
    let warnings = support::LevelLog::warnings();
    let guard = set_default(tracing_subscriber::registry().with(warnings.clone()));
    let service = service(
        settings(None, None, Some(vec!["gpt".to_owned()])),
        empty_limiter(),
        None,
    );
    let calls =
        r#"[{"function": "should_skip", "data": {}}, {"function": "should_skip", "data": {}}]"#;
    let answers = run_calls(&service, "{}", calls);
    drop(guard);
    assert!(skip_answer(answers[0].clone()).0);
    assert!(skip_answer(answers[1].clone()).0);
    assert_eq!(warnings.lines(), vec![WARNING.to_owned()]);
}

#[rstest]
#[case::unset(None)]
#[case::empty(Some(Vec::new()))]
fn unsupported_model_skip_warning_is_silent(#[case] models: Option<Vec<String>>) {
    let warnings = support::LevelLog::warnings();
    let guard = set_default(tracing_subscriber::registry().with(warnings.clone()));
    let service = service(settings(None, None, models), empty_limiter(), None);
    drop(run_calls(
        &service,
        "{}",
        r#"[{"function": "should_skip", "data": {}}]"#,
    ));
    drop(guard);
    assert!(warnings.lines().is_empty());
}

#[rstest]
#[case::wildcard(r#"{"models": ["*"]}"#, false)]
#[case::all_proxy(r#"{"models": ["all-proxy-models"]}"#, false)]
#[case::all_proxy_with_others(r#"{"models": ["all-proxy-models", "gpt"]}"#, false)]
#[case::access_group(r#"{"access_group_ids": ["group-1"]}"#, true)]
#[case::named_model(r#"{"models": ["gpt"]}"#, true)]
#[case::no_models("{}", false)]
#[case::empty_models(r#"{"models": []}"#, false)]
#[case::empty_groups(r#"{"access_group_ids": []}"#, false)]
fn key_requires_access_check(#[case] principal: &str, #[case] expected: bool) {
    let service = service(settings(None, None, None), empty_limiter(), None);
    let answer = single(run_calls(&service, principal, ACCESS_CHECK_CALL));
    assert_eq!(access_check_flag(&answer), expected);
}

fn base_descriptors() -> Vec<BatchLimitDescriptor> {
    vec![
        descriptor("api_key", "key-batch", Some(10), Some(100)),
        descriptor("user", "user-a", Some(5), None),
        descriptor("team", "team-1", Some(7), None),
        descriptor("end_user", "user-1", Some(9), None),
    ]
}

#[test]
fn rate_limit_descriptors() {
    let service = service(
        settings(None, None, None),
        limiter(base_descriptors(), Vec::new(), 1, 1024, false),
        None,
    );
    let answer = single(run_calls(
        &service,
        TPD_PRINCIPAL,
        r#"[{"function": "create_batch_rate_limit_descriptors", "data": {}}]"#,
    ));
    let descriptors = descriptors_answer(answer);
    let keys: Vec<&str> = descriptors.iter().map(|entry| entry.key.as_str()).collect();
    assert_eq!(
        keys,
        vec!["user", "api_key_tpd", "team_tpd", "end_user_tpd"]
    );
    let api_key = &descriptors[1];
    assert_eq!(api_key.value, "key-batch");
    assert_eq!(api_key.key, format!("api_key{TPD_SUFFIX}"));
    let rate_limit = api_key.rate_limit.as_ref().expect("a rate limit");
    assert_eq!(rate_limit.requests_per_unit, None);
    assert_eq!(rate_limit.tokens_per_unit, Some(1000));
    assert_eq!(rate_limit.window_size, Some(TPD_WINDOW_SECONDS));
    let team_limit = descriptors[2].rate_limit.as_ref().expect("a rate limit");
    assert_eq!(team_limit.tokens_per_unit, Some(0));
}

#[test]
fn rate_limit_descriptors_hash_the_api_key() {
    let service = service(
        settings(None, None, None),
        limiter(base_descriptors(), Vec::new(), 1, 1024, false),
        None,
    );
    let answer = single(run_calls(
        &service,
        r#"{"api_key": "sk-batch-key", "tpd_limit": 1000}"#,
        r#"[{"function": "create_batch_rate_limit_descriptors", "data": {}}]"#,
    ));
    let descriptors = descriptors_answer(answer);
    let tpd_key = format!("api_key{TPD_SUFFIX}");
    let api_key = descriptors
        .iter()
        .find(|descriptor| descriptor.key == tpd_key)
        .expect("the api key tpd descriptor");
    assert_eq!(api_key.value, safe_hash_api_key("sk-batch-key"));
}

#[test]
fn rate_limit_descriptors_without_tpd_limits() {
    let service = service(
        settings(None, None, None),
        limiter(base_descriptors(), Vec::new(), 1, 1024, false),
        None,
    );
    let answer = single(run_calls(
        &service,
        r#"{"api_key": "key-batch"}"#,
        r#"[{"function": "create_batch_rate_limit_descriptors", "data": {}}]"#,
    ));
    let descriptors = descriptors_answer(answer);
    let keys: Vec<&str> = descriptors.iter().map(|entry| entry.key.as_str()).collect();
    assert_eq!(keys, vec!["api_key", "user", "team", "end_user"]);
}

#[test]
fn rate_limit_descriptors_skip_empty_scope_values() {
    let service = service(
        settings(None, None, None),
        limiter(base_descriptors(), Vec::new(), 1, 1024, false),
        None,
    );
    let answer = single(run_calls(
        &service,
        r#"{"api_key": "key-batch", "team_id": "", "team_tpd_limit": 700}"#,
        r#"[{"function": "create_batch_rate_limit_descriptors", "data": {}}]"#,
    ));
    let descriptors = descriptors_answer(answer);
    let keys: Vec<&str> = descriptors.iter().map(|entry| entry.key.as_str()).collect();
    assert_eq!(keys, vec!["api_key", "user", "team", "end_user"]);
}

#[test]
fn limiter_failure_is_reported() {
    let service = service(
        settings(None, None, None),
        limiter(Vec::new(), Vec::new(), 1, 1024, true),
        None,
    );
    let outcome = service.run(&support::input(
        "{}",
        r#"[{"function": "create_batch_rate_limit_descriptors", "data": {}}]"#,
    ));
    assert!(matches!(outcome, Err(BatchLimitError::Limits(_))));
}

#[rstest]
#[case::no_limits("[]", false)]
#[case::requests(
    r#"[{"key": "api_key", "value": "v", "rate_limit": {"requests_per_unit": 5}}]"#,
    true
)]
#[case::tokens(
    r#"[{"key": "api_key", "value": "v", "rate_limit": {"tokens_per_unit": 5}}]"#,
    true
)]
#[case::parallel(
    r#"[{"key": "api_key", "value": "v", "rate_limit": {"max_parallel_requests": 1}}]"#,
    true
)]
#[case::empty_rate_limit(r#"[{"key": "api_key", "value": "v", "rate_limit": {}}]"#, false)]
#[case::null_rate_limit(r#"[{"key": "api_key", "value": "v", "rate_limit": null}]"#, false)]
fn has_applicable_limits(#[case] descriptors: &str, #[case] expected: bool) {
    let service = service(settings(None, None, None), empty_limiter(), None);
    let calls =
        format!(r#"[{{"function": "has_applicable_limits", "descriptors": {descriptors}}}]"#);
    let answer = single(run_calls(&service, "{}", &calls));
    assert_eq!(applicable_flag(&answer), expected);
}

fn skip_service(preset: &str) -> BatchLimitDescriptors<FakeLimiter, FakeCredentials> {
    let (config, descriptors, provider) = match preset {
        "disabled" => (settings(Some(true), None, None), Vec::new(), None),
        "providers" => (
            settings(None, Some(vec!["openai".to_owned()]), None),
            Vec::new(),
            Some("openai"),
        ),
        "provider-mismatch" => (
            settings(None, Some(vec!["openai".to_owned()]), None),
            Vec::new(),
            Some("anthropic"),
        ),
        "limits" => (
            settings(None, None, None),
            vec![descriptor("api_key", "key-batch", Some(10), None)],
            None,
        ),
        other => panic!("unknown preset {other}"),
    };
    let mut models = BTreeMap::new();
    let credentials = match provider {
        Some(name) => {
            let mut answer = support::credentials(name);
            answer.insert("api_key".to_owned(), Json::Text("secret".to_owned()));
            models.insert("bound-model".to_owned(), Some(answer.clone()));
            models.insert("top-model".to_owned(), Some(answer.clone()));
            router(models, Some(answer), false)
        }
        None => router(models, None, false),
    };
    service(
        config,
        limiter(descriptors, Vec::new(), 1, 1024, false),
        Some(credentials),
    )
}

#[rstest]
#[case::disabled("disabled", "{}", true)]
#[case::disabled_restricted("disabled", r#"{"models": ["gpt"]}"#, false)]
#[case::provider_skip("providers", "{}", true)]
#[case::provider_mismatch("provider-mismatch", "{}", true)]
fn should_skip_decision(#[case] preset: &str, #[case] principal: &str, #[case] expected: bool) {
    let service = skip_service(preset);
    let answer = single(run_calls(
        &service,
        principal,
        r#"[{"function": "should_skip", "data": {"model": "top-model"}}]"#,
    ));
    let (skip, descriptors) = skip_answer(answer);
    assert_eq!(skip, expected);
    assert_eq!(descriptors, None);
}

#[test]
fn should_skip_keeps_descriptors_when_limits_apply() {
    let service = skip_service("limits");
    let answer = single(run_calls(
        &service,
        r#"{"api_key": "key-batch"}"#,
        r#"[{"function": "should_skip", "data": {}}]"#,
    ));
    let (skip, descriptors) = skip_answer(answer);
    assert!(!skip);
    assert_eq!(descriptors.map(|found| found.len()), Some(1));
}

#[test]
fn should_skip_keeps_descriptors_when_project_limits_apply() {
    let service = service(settings(None, None, None), empty_limiter(), None);
    let answer = single(run_calls(
        &service,
        r#"{"project_id": "proj-1", "project_metadata": {"model_otpm_limit": {"gpt": 5}}}"#,
        r#"[{"function": "should_skip", "data": {}}]"#,
    ));
    let (skip, descriptors) = skip_answer(answer);
    assert!(!skip);
    assert_eq!(descriptors.map(|found| found.len()), Some(0));
}

#[test]
fn should_skip_runs_when_disabling_is_off() {
    let service = service(
        settings(Some(false), None, None),
        limiter(
            vec![descriptor("api_key", "key-batch", Some(10), None)],
            Vec::new(),
            1,
            1024,
            false,
        ),
        None,
    );
    let answer = single(run_calls(
        &service,
        r#"{"api_key": "key-batch"}"#,
        r#"[{"function": "should_skip", "data": {}}]"#,
    ));
    let (skip, descriptors) = skip_answer(answer);
    assert!(!skip);
    assert_eq!(descriptors.map(|found| found.len()), Some(1));
}

#[test]
fn should_skip_does_not_resolve_without_skip_providers() {
    let mut models = BTreeMap::new();
    models.insert("top-model".to_owned(), Some(support::credentials("openai")));
    let service = service(
        settings(None, None, None),
        empty_limiter(),
        Some(router(models, None, false)),
    );
    let answer = single(run_calls(
        &service,
        "{}",
        r#"[{"function": "should_skip", "data": {"model": "top-model"}}]"#,
    ));
    assert!(skip_answer(answer).0);
    assert!(
        service
            .credentials
            .as_ref()
            .expect("a router")
            .asked()
            .is_empty()
    );
}

#[test]
fn skip_debug_messages_name_the_provider_and_the_missing_limits() {
    let messages = support::LevelLog::debugs();
    let guard = set_default(tracing_subscriber::registry().with(messages.clone()));
    let providers = skip_service("providers");
    drop(run_calls(
        &providers,
        "{}",
        r#"[{"function": "should_skip", "data": {"model": "top-model"}}]"#,
    ));
    let bare = service(settings(None, None, None), empty_limiter(), None);
    drop(run_calls(
        &bare,
        "{}",
        r#"[{"function": "should_skip", "data": {}}]"#,
    ));
    drop(guard);
    let lines = messages.lines();
    assert!(lines.contains(&"Skipping batch input file processing for provider=openai".to_owned()));
    assert!(
        lines.contains(
            &"Skipping batch input file processing: no rate limits configured".to_owned()
        )
    );
}

#[test]
fn should_skip_enqueued_scopes_need_inspection() {
    let service = service(settings(None, None, None), empty_limiter(), None);
    let answer = single(run_calls(
        &service,
        "{}",
        r#"[{"function": "should_skip", "data": {}, "has_enqueued_scopes": true}]"#,
    ));
    let (skip, descriptors) = skip_answer(answer);
    assert!(!skip);
    assert_eq!(descriptors.map(|found| found.len()), Some(0));
}

#[test]
fn provider_skip_reads_the_file_bound_model() {
    let service = skip_service("providers");
    let file_id = encode_file_id_with_model("file-original", "bound-model", RoutingIdType::File);
    let data = format!(r#"{{"input_file_id": "{file_id}", "model": "absent-model"}}"#);
    let calls = format!(r#"[{{"function": "should_skip", "data": {data}}}]"#);
    let answer = single(run_calls(&service, "{}", &calls));
    assert!(skip_answer(answer).0);
    assert_eq!(
        service.credentials.as_ref().expect("a router").asked(),
        vec!["bound-model".to_owned()]
    );
}

#[test]
fn fetch_params_for_model_embedded_id() {
    let mut credentials = support::credentials("openai");
    credentials.insert("api_key".to_owned(), Json::Text("secret".to_owned()));
    credentials.insert(
        "aws_region_name".to_owned(),
        Json::Text("us-east-1".to_owned()),
    );
    credentials.insert("unrelated".to_owned(), Json::Text("dropped".to_owned()));
    let mut models = BTreeMap::new();
    models.insert("bound-model".to_owned(), Some(credentials));
    let service = service(
        settings(None, None, None),
        empty_limiter(),
        Some(router(models, None, false)),
    );
    let file_id = encode_file_id_with_model("file-original", "bound-model", RoutingIdType::File);
    let calls = format!(
        r#"[{{"function": "resolve_batch_input_file_fetch_params", "file_id": "{file_id}", "custom_llm_provider": "azure", "data": {{"model": "top-model"}}}}]"#
    );
    let answer = single(run_calls(&service, "{}", &calls));
    let (resolved_file_id, params) = fetch_params_answer(answer);
    assert_eq!(resolved_file_id, "file-original");
    let mut expected = JsonObject::new();
    expected.insert(
        "custom_llm_provider".to_owned(),
        Json::Text("openai".to_owned()),
    );
    expected.insert("model".to_owned(), Json::Text("bound-model".to_owned()));
    expected.insert("api_key".to_owned(), Json::Text("secret".to_owned()));
    expected.insert(
        "aws_region_name".to_owned(),
        Json::Text("us-east-1".to_owned()),
    );
    assert_eq!(params, expected);
}

#[test]
fn fetch_params_for_plain_file_id_uses_the_request_model() {
    let mut models = BTreeMap::new();
    models.insert(
        "top-model".to_owned(),
        Some(support::credentials("vertex_ai")),
    );
    let service = service(
        settings(None, None, None),
        empty_limiter(),
        Some(router(models, None, false)),
    );
    let answer = single(run_calls(
        &service,
        "{}",
        r#"[{"function": "resolve_batch_input_file_fetch_params", "file_id": "file-plain", "custom_llm_provider": "azure", "data": {"model": "top-model"}}]"#,
    ));
    let (resolved_file_id, params) = fetch_params_answer(answer);
    assert_eq!(resolved_file_id, "file-plain");
    assert_eq!(
        params.get("custom_llm_provider"),
        Some(&Json::Text("vertex_ai".to_owned()))
    );
    assert_eq!(
        params.get("model"),
        Some(&Json::Text("top-model".to_owned()))
    );
}

#[test]
fn fetch_params_without_a_router_keeps_the_base_provider() {
    let service = service(settings(None, None, None), empty_limiter(), None);
    let answer = single(run_calls(
        &service,
        "{}",
        r#"[{"function": "resolve_batch_input_file_fetch_params", "file_id": "file-plain", "custom_llm_provider": "azure", "data": {"model": "top-model"}}]"#,
    ));
    let (resolved_file_id, params) = fetch_params_answer(answer);
    assert_eq!(resolved_file_id, "file-plain");
    assert_eq!(params.len(), 1);
    assert_eq!(
        params.get("custom_llm_provider"),
        Some(&Json::Text("azure".to_owned()))
    );
}

#[test]
fn resolve_batch_provider_reads_trusted_credentials() {
    let mut models = BTreeMap::new();
    models.insert(
        "top-model".to_owned(),
        Some(support::credentials("bedrock")),
    );
    let service = service(
        settings(None, None, None),
        empty_limiter(),
        Some(router(models, None, false)),
    );
    let answer = single(run_calls(
        &service,
        "{}",
        r#"[{"function": "resolve_batch_provider", "model": "top-model"}]"#,
    ));
    assert_eq!(provider_answer(answer).as_deref(), Some("bedrock"));
}

#[test]
fn resolve_batch_provider_answers_none_without_a_router_or_model() {
    let service = service(settings(None, None, None), empty_limiter(), None);
    let answers = run_calls(
        &service,
        "{}",
        r#"[{"function": "resolve_batch_provider", "model": "top-model"}, {"function": "resolve_batch_provider", "model": null}]"#,
    );
    assert_eq!(answers.len(), 2);
    assert!(answers.iter().all(|answer| matches!(
        answer,
        BatchLimitAnswer::ResolveBatchProvider { provider: None }
    )));
}

#[test]
fn resolve_batch_provider_skips_an_empty_model() {
    let mut models = BTreeMap::new();
    models.insert("top-model".to_owned(), Some(support::credentials("openai")));
    let service = service(
        settings(None, None, None),
        empty_limiter(),
        Some(router(models, None, false)),
    );
    let answer = single(run_calls(
        &service,
        "{}",
        r#"[{"function": "resolve_batch_provider", "model": ""}]"#,
    ));
    assert_eq!(provider_answer(answer), None);
    assert!(
        service
            .credentials
            .as_ref()
            .expect("a router")
            .asked()
            .is_empty()
    );
}

#[test]
fn empty_provider_credential_is_absent() {
    let mut models = BTreeMap::new();
    models.insert("top-model".to_owned(), Some(support::credentials("")));
    let service = service(
        settings(None, None, None),
        empty_limiter(),
        Some(router(models, None, false)),
    );
    let answer = single(run_calls(
        &service,
        "{}",
        r#"[{"function": "resolve_batch_provider", "model": "top-model"}]"#,
    ));
    assert_eq!(provider_answer(answer), None);
}
