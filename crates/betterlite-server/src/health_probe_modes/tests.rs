#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::{DecisionsJson, Json};
use indexmap::IndexMap;
use rstest::rstest;

use self::support::{
    FakeBackend, FakeCheapestModels, FakeLogging, IMAGE_BYTES, bad_request_class,
    content_policy_class, context_window_class, evaluation_params_with_questions, probe_logging,
    probe_params, probe_params_from_json, run_mode, unclassified_class, unsupported_params_class,
    with_logging, with_prompt,
};
use super::batch::BATCH_PROVIDERS;
use super::calls::HealthProbeReply;
use super::error::HealthProbeError;
use super::evaluation::{HEALTH_CHECK_QUESTION, evaluation_questions};
use super::image_data::{IMAGE_EDIT_HEALTH_CHECK_PROMPT, health_check_image};
use super::logging::{HealthProbeLogging, ProbeLoggingUpdate};
use super::params::Param;
use super::plan::{HealthProbeArguments, plan_health_probe};
use super::response::{
    HealthCheckResponse, REGION_HEADER, REMAINING_REQUESTS_HEADER, REMAINING_TOKENS_HEADER,
    health_check_response,
};
use super::run::run_health_probe;
use super::tracking::{SERVICE_ACCOUNT, add_tracking_information};
use super::wildcard::probe_wildcard_model;

mod support;

include!("tests/modes.rs");

#[test]
fn evaluation_hosted_vllm_question_is_a_choice() {
    let questions = evaluation_questions("hosted_vllm");
    let Some(Json::Object(question)) = questions.get("reachable") else {
        panic!("the health check question is an object");
    };
    assert_eq!(question.get("type"), Some(&Json::Text("choice".to_owned())));
    assert_eq!(
        question.get("instructions"),
        Some(&Json::Text(HEALTH_CHECK_QUESTION.to_owned()))
    );
    let Some(Json::Object(criteria)) = question.get("criteria") else {
        panic!("the choice question carries criteria");
    };
    assert_eq!(criteria.get("yes"), Some(&Json::Null));
    assert_eq!(criteria.get("no"), Some(&Json::Null));
}

#[rstest]
#[case::openai("openai")]
#[case::bedrock("bedrock")]
#[case::unknown("unknown_provider")]
#[test]
fn evaluation_other_providers_get_a_noul_question(#[case] provider: &str) {
    let questions = evaluation_questions(provider);
    let Some(Json::Object(question)) = questions.get("reachable") else {
        panic!("the health check question is an object");
    };
    assert_eq!(question.get("type"), Some(&Json::Text("noul".to_owned())));
    assert_eq!(
        question.get("instructions"),
        Some(&Json::Text(HEALTH_CHECK_QUESTION.to_owned()))
    );
    assert!(!question.contains_key("criteria"));
}

#[rstest]
#[case::missing(None, "health check")]
#[case::empty(Some(""), "health check")]
#[case::given(Some("is it reachable?"), "is it reachable?")]
#[tokio::test]
async fn evaluation_state_uses_the_given_prompt_or_the_default(
    #[case] prompt: Option<&str>,
    #[case] expected: &str,
) {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(r#"{"model":"m","messages":[]}"#);
    run_mode(
        "evaluation",
        "openai",
        params,
        with_prompt(prompt),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_evaluation();
    assert_eq!(call.params.state, DecisionsJson::Text(expected.to_owned()));
}

#[tokio::test]
async fn evaluation_hosted_vllm_plan_builds_the_choice_questions() {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(r#"{"model":"m"}"#);
    run_mode(
        "evaluation",
        "hosted_vllm",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_evaluation();
    let questions = serde_json::to_string(&call.params.questions).unwrap();
    assert!(questions.contains(r#""type":"choice""#));
    assert!(questions.contains(r#""criteria""#));
}

#[tokio::test]
async fn evaluation_parameters_override_the_built_state_and_questions() {
    let backend = FakeBackend::new();
    let entries = concat!(
        r#"{"model":"m","state":{"custom":true},"#,
        r#""questions":{"custom":{"type":"noul","instructions":"custom question"}}}"#
    );
    let params = probe_params_from_json(entries);
    run_mode(
        "evaluation",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_evaluation();
    let state = serde_json::to_string(&call.params.state).unwrap();
    assert_eq!(state, r#"{"custom":true}"#);
    let questions = serde_json::to_string(&call.params.questions).unwrap();
    assert_eq!(
        questions,
        r#"{"custom":{"type":"noul","instructions":"custom question"}}"#
    );
}

#[tokio::test]
async fn evaluation_optional_fields_reach_the_call() {
    let backend = FakeBackend::new();
    let entries = concat!(
        r#"{"model":"m","api_key":"sk-test","api_base":"https://decisions.test","#,
        r#""custom_llm_provider":"hosted_vllm","timeout":16,"#,
        r#""extra_headers":{"x-team":"health"},"top_k":5}"#
    );
    let params = probe_params_from_json(entries);
    run_mode(
        "evaluation",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_evaluation();
    assert_eq!(
        call.params.api_base.as_deref(),
        Some("https://decisions.test")
    );
    assert_eq!(
        call.params.custom_llm_provider.as_deref(),
        Some("hosted_vllm")
    );
    assert_eq!(call.params.timeout, Some(16.0_f64));
    let headers = serde_json::to_string(&call.params.extra_headers).unwrap();
    assert_eq!(headers, r#"{"x-team":"health"}"#);
    let extra = serde_json::to_string(&call.params.extra).unwrap();
    assert_eq!(extra, r#"{"top_k":5}"#);
    assert_eq!(call.params.api_key.unwrap().expose_secret(), "sk-test");
}

#[tokio::test]
async fn evaluation_payload_without_a_model_key_is_rejected() {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(r#"{"state":"health check"}"#);
    let error = run_mode(
        "evaluation",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        HealthProbeError::EvaluationPayload { field: "model", .. }
    ));
    assert_eq!(backend.call_count(), 0);
}

#[rstest]
#[case::state_is_a_bool(r#"{"model":"m","state":true}"#, "state")]
#[case::state_is_null(r#"{"model":"m","state":null}"#, "state")]
#[case::questions_is_a_list(r#"{"model":"m","questions":[]}"#, "questions")]
#[case::questions_is_empty(r#"{"model":"m","questions":{}}"#, "questions")]
#[case::timeout_is_a_string(r#"{"model":"m","timeout":"16"}"#, "timeout")]
#[case::headers_is_a_number(r#"{"model":"m","extra_headers":3}"#, "extra_headers")]
#[case::header_value_is_a_number(r#"{"model":"m","extra_headers":{"x-team":1}}"#, "extra_headers")]
#[tokio::test]
async fn evaluation_invalid_payloads_are_rejected(#[case] entries: &str, #[case] field: &str) {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(entries);
    let error = run_mode(
        "evaluation",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        HealthProbeError::EvaluationPayload { field: reported, .. } if reported == field
    ));
    assert_eq!(backend.call_count(), 0);
}

#[tokio::test]
async fn evaluation_accepts_128_questions() {
    let backend = FakeBackend::new();
    let params = evaluation_params_with_questions(128);
    run_mode(
        "evaluation",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    assert_eq!(backend.call_count(), 1);
}

#[tokio::test]
async fn evaluation_rejects_129_questions() {
    let backend = FakeBackend::new();
    let params = evaluation_params_with_questions(129);
    let error = run_mode(
        "evaluation",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        HealthProbeError::EvaluationPayload {
            field: "questions",
            ..
        }
    ));
    assert_eq!(backend.call_count(), 0);
}

#[tokio::test]
async fn wildcard_uses_the_cheapest_models_and_null_fallbacks_for_one() {
    let backend = FakeBackend::new();
    let cheapest = FakeCheapestModels::new(&["cheap-a", "cheap-b", "cheap-c"]);
    let params = probe_params_from_json(r#"{"model":"openai/*"}"#);
    let response = probe_wildcard_model("openai/*", "openai", params, &cheapest, &backend)
        .await
        .unwrap();
    assert_eq!(response, HealthCheckResponse::default());
    let call = backend.take_chat();
    assert_eq!(
        call.parameters.get("model"),
        Some(&Json::Text("cheap-a".to_owned()))
    );
    let fallbacks = serde_json::to_string(call.parameters.get("fallbacks").unwrap()).unwrap();
    assert_eq!(fallbacks, r#"["cheap-b","cheap-c"]"#);
    let max_tokens = serde_json::to_string(call.parameters.get("max_tokens").unwrap()).unwrap();
    assert_eq!(max_tokens, "16");
    let request = cheapest.request.lock().unwrap().clone();
    assert_eq!(request, Some(("openai".to_owned(), 3_usize)));
    let single_backend = FakeBackend::new();
    let single_cheapest = FakeCheapestModels::new(&["only-cheap"]);
    let single_params = probe_params_from_json(r#"{"model":"*"}"#);
    probe_wildcard_model(
        "*",
        "openai",
        single_params,
        &single_cheapest,
        &single_backend,
    )
    .await
    .unwrap();
    assert_eq!(
        single_backend.take_chat().parameters.get("fallbacks"),
        Some(&Json::Null)
    );
}

#[rstest]
#[case::given(r#"{"model":"*","max_tokens":5}"#, "5")]
#[case::zero(r#"{"model":"*","max_tokens":0}"#, "0")]
#[case::null(r#"{"model":"*","max_tokens":null}"#, "null")]
#[tokio::test]
async fn wildcard_keeps_a_given_max_tokens(#[case] entries: &str, #[case] expected: &str) {
    let backend = FakeBackend::new();
    let cheapest = FakeCheapestModels::new(&["only-cheap"]);
    let params = probe_params_from_json(entries);
    probe_wildcard_model("*", "openai", params, &cheapest, &backend)
        .await
        .unwrap();
    let call = backend.take_chat();
    let max_tokens = serde_json::to_string(call.parameters.get("max_tokens").unwrap()).unwrap();
    assert_eq!(max_tokens, expected);
}

#[tokio::test]
async fn wildcard_without_models_fails_with_the_exact_message() {
    let backend = FakeBackend::new();
    let cheapest = FakeCheapestModels::new(&[]);
    let params = probe_params_from_json(r#"{"model":"*"}"#);
    let error = probe_wildcard_model("*", "openai", params, &cheapest, &backend)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Unable to health check wildcard model for provider openai. Add a model on your config.yaml or contribute here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json"
    );
    assert_eq!(backend.call_count(), 0);
}

#[tokio::test]
async fn wildcard_returns_an_empty_response_and_ignores_headers() {
    let mut backend = FakeBackend::new();
    backend
        .headers
        .insert(REGION_HEADER.to_owned(), "eastus".to_owned());
    let logging: Arc<dyn HealthProbeLogging> = Arc::new(FakeLogging::new(backend.events(), false));
    let params = with_logging(probe_params_from_json(r#"{"model":"*"}"#), logging);
    let cheapest = FakeCheapestModels::new(&["only-cheap"]);
    let response = probe_wildcard_model("*", "openai", params, &cheapest, &backend)
        .await
        .unwrap();
    assert_eq!(response, HealthCheckResponse::default());
    assert!(backend.take_chat().logging.is_some());
}

#[test]
fn tracking_replaces_an_earlier_metadata_value() {
    let entries = concat!(
        r#"{"model":"m","litellm_metadata":{"tags":["old"],"#,
        r#""user_api_key_auth":{"api_key":"k"}}}"#
    );
    let updated = add_tracking_information(probe_params_from_json(entries)).unwrap();
    let Param::Value(Json::Object(metadata)) = updated.litellm_metadata else {
        panic!("the tracking metadata is an object");
    };
    let tags = serde_json::to_string(metadata.get("tags").unwrap()).unwrap();
    let expected = format!(r#"["{SERVICE_ACCOUNT}"]"#);
    assert_eq!(tags, expected);
    assert!(!metadata.contains_key("user_api_key_auth"));
}

#[test]
fn tracking_applies_the_internal_health_principal() {
    let params = probe_params_from_json(r#"{"model":"m"}"#);
    let updated = add_tracking_information(params).unwrap();
    assert_eq!(updated.model, Param::Value("m".to_owned()));
    let Param::Value(Json::Object(metadata)) = updated.litellm_metadata else {
        panic!("the tracking metadata is an object");
    };
    let name = Json::Text(SERVICE_ACCOUNT.to_owned());
    assert_eq!(metadata.get("user_api_key"), Some(&name));
    assert_eq!(metadata.get("user_api_key_hash"), Some(&name));
    assert_eq!(metadata.get("user_api_key_alias"), Some(&name));
    assert_eq!(metadata.get("user_api_key_team_id"), Some(&name));
    assert_eq!(metadata.get("user_api_key_team_alias"), Some(&name));
    let spend = serde_json::to_string(metadata.get("user_api_key_spend").unwrap()).unwrap();
    assert_eq!(spend, "0.0");
    assert!(!metadata.contains_key("user_api_key_auth"));
}

#[rstest]
#[case::openai("openai")]
#[case::azure("azure")]
#[case::hosted_vllm("hosted_vllm")]
#[case::litellm_proxy("litellm_proxy")]
#[case::vertex_ai("vertex_ai")]
#[case::xai("xai")]
#[tokio::test]
async fn batch_supported_providers_force_the_provider_on_list_batches(#[case] provider: &str) {
    let backend = FakeBackend::new();
    let entries = concat!(
        r#"{"model":"m","messages":[{"role":"user","content":"hi"}],"#,
        r#""custom_llm_provider":"given"}"#
    );
    let params = probe_params_from_json(entries);
    run_mode(
        "batch",
        provider,
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_batch();
    assert_eq!(
        call.parameters.get("custom_llm_provider"),
        Some(&Json::Text(provider.to_owned()))
    );
    assert!(!call.parameters.contains_key("messages"));
}

#[test]
fn batch_supported_provider_list_matches_litellm() {
    assert_eq!(
        BATCH_PROVIDERS,
        [
            "openai",
            "azure",
            "hosted_vllm",
            "litellm_proxy",
            "vertex_ai",
            "xai"
        ]
    );
}

#[rstest]
#[case::bedrock("bedrock")]
#[case::groq("groq")]
#[tokio::test]
async fn batch_other_providers_send_the_full_chat_call(#[case] provider: &str) {
    let backend = FakeBackend::new();
    let entries = concat!(
        r#"{"model":"m","messages":[{"role":"user","content":"hi"}],"#,
        r#""custom_llm_provider":"given"}"#
    );
    let params = probe_params_from_json(entries);
    run_mode(
        "batch",
        provider,
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_chat();
    assert!(call.parameters.contains_key("messages"));
    assert_eq!(
        call.parameters.get("custom_llm_provider"),
        Some(&Json::Text("given".to_owned()))
    );
}

#[tokio::test]
async fn batch_updates_the_logging_object_before_the_call() {
    let backend = FakeBackend::new();
    let logging = Arc::new(FakeLogging::new(backend.events(), false));
    let entries =
        r#"{"model":"m","api_base":"https://api.test","messages":[],"api_key":"sk-test"}"#;
    let params = probe_params_from_json(entries);
    run_mode(
        "batch",
        "openai",
        with_logging(params, Arc::clone(&logging)),
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    let events = logging.events.lock().unwrap().clone();
    assert_eq!(events, ["logging", "call"]);
    let updates = logging.take_updates();
    assert_eq!(updates.len(), 1);
    let update = &updates[0];
    assert_eq!(update.model.as_deref(), Some("m"));
    assert_eq!(update.user, None);
    assert!(update.optional_params.is_empty());
    let api_base = Json::Text("https://api.test".to_owned());
    assert_eq!(update.kwargs.get("api_base"), Some(&api_base));
    assert!(!update.kwargs.contains_key("messages"));
    assert!(!update.kwargs.contains_key("api_key"));
    let litellm_params = update.litellm_params.as_ref().unwrap();
    assert_eq!(litellm_params.get("api_base"), Some(&api_base));
    assert!(backend.take_batch().logging.is_some());
}

#[rstest]
#[case::absent(r#"{"model":"m"}"#)]
#[case::null(r#"{"model":"m","api_base":null}"#)]
#[case::empty(r#"{"model":"m","api_base":""}"#)]
#[tokio::test]
async fn batch_logging_update_skips_a_falsy_api_base(#[case] entries: &str) {
    let backend = FakeBackend::new();
    let logging = Arc::new(FakeLogging::new(backend.events(), false));
    let params = probe_params_from_json(entries);
    run_mode(
        "batch",
        "openai",
        with_logging(params, Arc::clone(&logging)),
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    let updates = logging.take_updates();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].litellm_params, None);
    assert_eq!(updates[0].model.as_deref(), Some("m"));
}

#[tokio::test]
async fn batch_logging_failure_stops_before_the_call() {
    let backend = FakeBackend::new();
    let logging: Arc<dyn HealthProbeLogging> = Arc::new(FakeLogging::new(backend.events(), true));
    let params = with_logging(probe_params_from_json(r#"{"model":"m"}"#), logging);
    let error = run_mode(
        "batch",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, HealthProbeError::Logging { .. }));
    assert_eq!(backend.call_count(), 0);
}

#[tokio::test]
async fn image_edit_sends_the_health_image_and_prompt() {
    let backend = FakeBackend::new();
    let entries = r#"{"model":"gpt-image-1","messages":[],"api_key":"sk-test"}"#;
    let params = probe_params_from_json(entries);
    let reply = run_mode(
        "image_edit",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    assert_eq!(reply, HealthProbeReply::default());
    let call = backend.take_image_edit();
    assert_eq!(call.prompt, IMAGE_EDIT_HEALTH_CHECK_PROMPT);
    assert_eq!(call.image.len(), IMAGE_BYTES);
    assert_eq!(call.image, health_check_image().unwrap());
    assert!(!call.parameters.contains_key("messages"));
    assert_eq!(call.api_key.unwrap().expose_secret(), "sk-test");
}

#[rstest]
#[case::content_policy(
    content_policy_class("the request was flagged"),
    "the request was flagged"
)]
#[case::bad_request_message(
    bad_request_class("moderation_blocked: policy"),
    "moderation_blocked: policy"
)]
#[case::context_window(
    context_window_class("blocked: moderation_blocked"),
    "blocked: moderation_blocked"
)]
#[case::unsupported_params(unsupported_params_class("moderation_blocked"), "moderation_blocked")]
#[tokio::test]
async fn image_edit_returns_an_empty_response_for_moderation_failures(
    #[case] class: ErrorClass,
    #[case] message: &str,
) {
    let backend = FakeBackend::new();
    backend.fail_image_edit(class, message);
    let params = probe_params_from_json(r#"{"model":"gpt-image-1"}"#);
    let reply = run_mode(
        "image_edit",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    assert_eq!(reply, HealthProbeReply::default());
    assert_eq!(backend.call_count(), 1);
}

#[rstest]
#[case::bad_request(bad_request_class("model not found"), "model not found")]
#[case::unclassified_moderation(unclassified_class("moderation_blocked"), "moderation_blocked")]
#[tokio::test]
async fn image_edit_returns_other_failures_unchanged(
    #[case] class: ErrorClass,
    #[case] message: &str,
) {
    let backend = FakeBackend::new();
    let expected = class.clone();
    backend.fail_image_edit(class, message);
    let params = probe_params_from_json(r#"{"model":"gpt-image-1"}"#);
    let error = run_mode(
        "image_edit",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), message);
    assert!(matches!(
        error,
        HealthProbeError::Backend { class: reported, .. } if reported == expected
    ));
}

#[test]
fn response_keeps_only_the_three_rate_limit_and_region_headers() {
    let mut headers = BTreeMap::new();
    headers.insert(REMAINING_REQUESTS_HEADER.to_owned(), "10".to_owned());
    headers.insert(REMAINING_TOKENS_HEADER.to_owned(), "20".to_owned());
    headers.insert(REGION_HEADER.to_owned(), "eastus".to_owned());
    headers.insert("x-other-header".to_owned(), "gone".to_owned());
    let response = health_check_response(&headers);
    let serialized = serde_json::to_string(&response).unwrap();
    assert_eq!(
        serialized,
        r#"{"x-ratelimit-remaining-requests":"10","x-ratelimit-remaining-tokens":"20","x-ms-region":"eastus"}"#
    );
}

#[test]
fn response_keeps_an_empty_header_value() {
    let mut headers = BTreeMap::new();
    headers.insert(REGION_HEADER.to_owned(), String::new());
    let response = health_check_response(&headers);
    assert_eq!(
        serde_json::to_string(&response).unwrap(),
        r#"{"x-ms-region":""}"#
    );
}

#[test]
fn response_is_an_empty_object_without_the_headers() {
    let response = health_check_response(&BTreeMap::new());
    assert_eq!(serde_json::to_string(&response).unwrap(), "{}");
}

#[test]
fn response_header_names_are_matched_in_lowercase_only() {
    let mut headers = BTreeMap::new();
    headers.insert("X-Ms-Region".to_owned(), "eastus".to_owned());
    headers.insert("X-RateLimit-Remaining-Requests".to_owned(), "1".to_owned());
    let response = health_check_response(&headers);
    assert_eq!(serde_json::to_string(&response).unwrap(), "{}");
}

#[test]
fn response_filtered_parameters_remove_messages() {
    let entries = concat!(
        r#"{"model":"gpt-5","messages":[{"role":"user","content":"hi"}],"#,
        r#""temperature":0.2}"#
    );
    let params = probe_params_from_json(entries);
    let full = params.call_parameters(true);
    let filtered = params.call_parameters(false);
    assert!(full.contains_key("messages"));
    assert!(!filtered.contains_key("messages"));
    assert_eq!(filtered.get("temperature"), full.get("temperature"));
}

#[test]
fn response_filtered_parameters_drop_a_raw_messages_key() {
    let mut params = probe_params_from_json(r#"{"model":"gpt-5"}"#);
    params
        .parameters
        .insert("messages".to_owned(), serde_json::json!([]));
    assert!(!params.call_parameters(false).contains_key("messages"));
    assert!(params.call_parameters(true).contains_key("messages"));
}

#[test]
fn response_health_image_is_a_512_by_512_png() {
    let image = health_check_image().unwrap();
    assert_eq!(image.len(), IMAGE_BYTES);
    assert_eq!(&image[..8], b"\x89PNG\r\n\x1a\n".as_slice());
    let header = &image[12..24];
    assert_eq!(header.len(), 12);
    assert_eq!(&header[..4], b"IHDR".as_slice());
    let width: [u8; 4] = header[4..8].try_into().unwrap();
    let height: [u8; 4] = header[8..12].try_into().unwrap();
    assert_eq!(
        (u32::from_be_bytes(width), u32::from_be_bytes(height)),
        (512, 512)
    );
}

#[tokio::test]
async fn response_run_returns_the_backend_headers_or_nothing() {
    let mut backend = FakeBackend::new();
    backend
        .headers
        .insert(REGION_HEADER.to_owned(), "westus".to_owned());
    let params = probe_params_from_json(r#"{"model":"gpt-5"}"#);
    let reply = run_mode(
        "chat",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    assert_eq!(
        reply.hidden_headers.get(REGION_HEADER).map(String::as_str),
        Some("westus")
    );
    let empty = FakeBackend::new();
    let empty_params = probe_params_from_json(r#"{"model":"gpt-5"}"#);
    let empty_reply = run_mode(
        "chat",
        "openai",
        empty_params,
        HealthProbeArguments::default(),
        &empty,
    )
    .await
    .unwrap();
    assert!(empty_reply.hidden_headers.is_empty());
}

#[test]
fn logging_object_records_the_health_update() {
    let logging = probe_logging();
    let update = ProbeLoggingUpdate {
        kwargs: IndexMap::new(),
        model: Some("health-model".to_owned()),
        user: Some("health-check".to_owned()),
        optional_params: IndexMap::new(),
        litellm_params: None,
    };
    logging.update_from_kwargs(update).unwrap();
    let guard = logging
        .logging_state()
        .expect("the health logging state is present");
    let state = guard.lock().unwrap();
    assert_eq!(state.model, "health-model");
    assert_eq!(state.user.as_deref(), Some("health-check"));
}
