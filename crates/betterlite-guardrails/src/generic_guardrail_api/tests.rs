#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::routing::post;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderName};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::{GenericGuardrailAPI, GenericGuardrailAPIError, extract_user_api_key_metadata};
use crate::generic_guardrail_contracts::{
    GenericGuardrailAPIMetadata, GenericGuardrailAPIRequest, GenericGuardrailAPIResponse,
    GenericGuardrailAPISettings, GuardrailExtraValue, GuardrailInputType, GuardrailMessage,
    GuardrailOptions, GuardrailResponseAction, UnreachableFallback,
};

const DEFAULT_PATH: &str = "/beta/litellm_basic_guardrail_api";

fn settings(
    fail_on_error: bool,
    unreachable_fallback: UnreachableFallback,
) -> GenericGuardrailAPISettings {
    GenericGuardrailAPISettings::from_optional(Some(GuardrailOptions {
        unreachable_fallback: Some(unreachable_fallback),
        fail_on_error: Some(fail_on_error),
        provider_params: None,
        streaming_end_of_stream_only: None,
        streaming_sampling_rate: None,
        streaming_transform_mode: None,
    }))
}

fn api(settings: GenericGuardrailAPISettings, api_base: &str) -> GenericGuardrailAPI {
    GenericGuardrailAPI {
        api_key: Some(SecretString::from("secret-token")),
        api_base: api_base.to_owned(),
        settings,
        provider_params: BTreeMap::new(),
        client: reqwest::Client::default(),
        timeout: super::DEFAULT_TIMEOUT,
    }
}

fn base_request() -> GenericGuardrailAPIRequest {
    GenericGuardrailAPIRequest {
        input_type: GuardrailInputType::Request,
        litellm_call_id: None,
        litellm_trace_id: None,
        structured_messages: None,
        images: None,
        tools: None,
        texts: None,
        request_data: GenericGuardrailAPIMetadata::default(),
        request_headers: None,
        litellm_version: None,
        provider_params: None,
        tool_calls: None,
        model: None,
    }
}

fn base_response() -> GenericGuardrailAPIResponse {
    GenericGuardrailAPIResponse {
        action: GuardrailResponseAction::None,
        texts: None,
        images: None,
        tools: None,
        structured_messages: None,
        blocked_reason: None,
        stream_holdback_chars: None,
    }
}

fn row(content: &str) -> GuardrailMessage {
    GuardrailMessage {
        role: "user".to_owned(),
        extra: BTreeMap::from([(
            "content".to_owned(),
            GuardrailExtraValue::Text(content.to_owned()),
        )]),
    }
}

#[test]
fn generic_guardrail_metadata_merges_litellm_metadata_first() {
    let metadata = BTreeMap::from([
        ("user_api_key_hash".to_owned(), Some("old".to_owned())),
        ("user_api_key_team_id".to_owned(), Some("team".to_owned())),
    ]);
    let litellm = BTreeMap::from([("user_api_key_hash".to_owned(), Some("preferred".to_owned()))]);
    let result = extract_user_api_key_metadata(&litellm, &metadata);
    assert_eq!(result.user_api_key_hash.as_deref(), Some("preferred"));
    assert_eq!(result.user_api_key_team_id.as_deref(), Some("team"));
}
#[test]
fn generic_guardrail_metadata_token_alias_overrides_hash() {
    let metadata = BTreeMap::from([
        ("user_api_key_hash".to_owned(), Some("hash".to_owned())),
        ("user_api_key_token".to_owned(), Some("token".to_owned())),
    ]);
    let result = extract_user_api_key_metadata(&BTreeMap::new(), &metadata);
    assert_eq!(result.user_api_key_hash.as_deref(), Some("token"));
}
#[test]
fn generic_guardrail_metadata_maps_every_known_field_and_drops_unknown_keys() {
    let metadata = BTreeMap::from([
        ("user_api_key_hash".to_owned(), Some("hash".to_owned())),
        ("user_api_key_alias".to_owned(), Some("alias".to_owned())),
        ("user_api_key_user_id".to_owned(), Some("user".to_owned())),
        (
            "user_api_key_user_email".to_owned(),
            Some("email".to_owned()),
        ),
        ("user_api_key_team_id".to_owned(), Some("team".to_owned())),
        (
            "user_api_key_team_alias".to_owned(),
            Some("team-alias".to_owned()),
        ),
        (
            "user_api_key_end_user_id".to_owned(),
            Some("end-user".to_owned()),
        ),
        ("user_api_key_org_id".to_owned(), Some("org".to_owned())),
        ("user_api_key_custom".to_owned(), Some("custom".to_owned())),
        ("unrelated".to_owned(), Some("dropped".to_owned())),
    ]);
    let result = extract_user_api_key_metadata(&BTreeMap::new(), &metadata);
    assert_eq!(result.user_api_key_hash.as_deref(), Some("hash"));
    assert_eq!(result.user_api_key_alias.as_deref(), Some("alias"));
    assert_eq!(result.user_api_key_user_id.as_deref(), Some("user"));
    assert_eq!(result.user_api_key_user_email.as_deref(), Some("email"));
    assert_eq!(result.user_api_key_team_id.as_deref(), Some("team"));
    assert_eq!(
        result.user_api_key_team_alias.as_deref(),
        Some("team-alias")
    );
    assert_eq!(result.user_api_key_end_user_id.as_deref(), Some("end-user"));
    assert_eq!(result.user_api_key_org_id.as_deref(), Some("org"));
    assert!(result.extra.is_empty());
}
#[test]
fn generic_guardrail_merge_provider_params_adds_the_configured_base_values() {
    let mut instance = api(
        GenericGuardrailAPISettings::from_optional(None),
        "http://example",
    );
    instance.provider_params = BTreeMap::from([(
        "custom_threshold".to_owned(),
        GuardrailExtraValue::Decimal(0.8),
    )]);
    let merged = instance.merge_provider_params(base_request());
    assert_eq!(
        merged.provider_params,
        Some(BTreeMap::from([(
            "custom_threshold".to_owned(),
            GuardrailExtraValue::Decimal(0.8)
        )]))
    );
}
#[test]
fn generic_guardrail_merge_provider_params_lets_the_request_override_the_base_value() {
    let mut instance = api(
        GenericGuardrailAPISettings::from_optional(None),
        "http://example",
    );
    instance.provider_params = BTreeMap::from([(
        "custom_threshold".to_owned(),
        GuardrailExtraValue::Decimal(0.8),
    )]);
    let request = GenericGuardrailAPIRequest {
        provider_params: Some(BTreeMap::from([(
            "custom_threshold".to_owned(),
            GuardrailExtraValue::Decimal(0.3),
        )])),
        ..base_request()
    };
    let merged = instance.merge_provider_params(request);
    assert_eq!(
        merged.provider_params,
        Some(BTreeMap::from([(
            "custom_threshold".to_owned(),
            GuardrailExtraValue::Decimal(0.3)
        )]))
    );
}
#[test]
fn generic_guardrail_merge_provider_params_sends_an_empty_object_when_nothing_is_configured() {
    let instance = api(
        GenericGuardrailAPISettings::from_optional(None),
        "http://example",
    );
    let merged = instance.merge_provider_params(base_request());
    assert_eq!(merged.provider_params, Some(BTreeMap::new()));
    let serialized = serde_json::to_value(&merged).expect("serialize");
    assert!(
        serialized["additional_provider_specific_params"]
            .as_object()
            .expect("object")
            .is_empty()
    );
}
#[derive(serde::Deserialize)]
struct CapturedProviderParams {
    #[serde(rename = "additional_provider_specific_params")]
    provider_params: BTreeMap<String, GuardrailExtraValue>,
}

#[tokio::test]
async fn generic_guardrail_apply_guardrail_merges_configured_provider_params() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}{}", listener.local_addr().unwrap(), DEFAULT_PATH);
    let mut instance = api(GenericGuardrailAPISettings::from_optional(None), &base);
    instance.provider_params = BTreeMap::from([(
        "custom_threshold".to_owned(),
        GuardrailExtraValue::Text("strict".to_owned()),
    )]);
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_for_handler = Arc::clone(&captured);
    let answer = move |body: Bytes| {
        let captured_for_body = Arc::clone(&captured_for_handler);
        async move {
            *captured_for_body.lock().unwrap() = body.to_vec();
            "{}".to_owned()
        }
    };
    let router = Router::new().route(DEFAULT_PATH, post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    instance
        .apply_guardrail(base_request())
        .await
        .expect("success");
    let body = captured.lock().unwrap().clone();
    let captured_params: CapturedProviderParams =
        serde_json::from_slice(&body).expect("captured payload is valid JSON");
    assert_eq!(
        captured_params.provider_params,
        BTreeMap::from([(
            "custom_threshold".to_owned(),
            GuardrailExtraValue::Text("strict".to_owned())
        )])
    );
}
#[test]
fn generic_guardrail_error_policy_is_explicit() {
    let settings = GenericGuardrailAPISettings::from_optional(Some(GuardrailOptions {
        unreachable_fallback: Some(UnreachableFallback::FailOpen),
        fail_on_error: Some(false),
        provider_params: None,
        streaming_end_of_stream_only: None,
        streaming_sampling_rate: None,
        streaming_transform_mode: None,
    }));
    assert!(!settings.fail_on_error);
    assert_eq!(settings.unreachable_fallback, UnreachableFallback::FailOpen);
}
#[test]
fn generic_guardrail_fail_on_error_override_unreachable() {
    let instance = api(
        settings(false, UnreachableFallback::FailClosed),
        "http://example",
    );
    let error = GenericGuardrailAPIError::HttpStatus {
        status: StatusCode::INTERNAL_SERVER_ERROR,
    };
    let fallback = GenericGuardrailAPIResponse {
        texts: Some(vec!["fallback".to_owned()]),
        ..base_response()
    };
    let result = instance.handle_error(error, fallback).expect("fallback");
    assert_eq!(result.texts, Some(vec!["fallback".to_owned()]));
}
#[test]
fn generic_guardrail_status_code_is_always_bad_request() {
    let error = GenericGuardrailAPIError::Blocked {
        reason: String::new(),
    };
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
}
#[test]
fn generic_guardrail_handle_error_propagates_a_reachable_error_with_fail_open_configured() {
    let instance = api(
        settings(true, UnreachableFallback::FailOpen),
        "http://example",
    );
    let error = GenericGuardrailAPIError::Blocked {
        reason: String::new(),
    };
    let fallback = GenericGuardrailAPIResponse {
        texts: Some(vec!["fallback".to_owned()]),
        ..base_response()
    };
    let result = instance.handle_error(error, fallback);
    assert!(matches!(
        result,
        Err(GenericGuardrailAPIError::Blocked { .. })
    ));
}
#[test]
fn generic_guardrail_handle_error_propagates_an_unreachable_error_with_fail_closed_configured() {
    let instance = api(
        settings(true, UnreachableFallback::FailClosed),
        "http://example",
    );
    let error = GenericGuardrailAPIError::HttpStatus {
        status: StatusCode::SERVICE_UNAVAILABLE,
    };
    let fallback = GenericGuardrailAPIResponse {
        texts: Some(vec!["fallback".to_owned()]),
        ..base_response()
    };
    let result = instance.handle_error(error, fallback);
    assert!(matches!(
        result,
        Err(GenericGuardrailAPIError::HttpStatus { .. })
    ));
}
#[test]
fn generic_guardrail_handle_error_falls_open_for_an_unreachable_error_with_fail_open_configured() {
    let instance = api(
        settings(true, UnreachableFallback::FailOpen),
        "http://example",
    );
    let error = GenericGuardrailAPIError::HttpStatus {
        status: StatusCode::SERVICE_UNAVAILABLE,
    };
    let fallback = GenericGuardrailAPIResponse {
        texts: Some(vec!["fallback".to_owned()]),
        ..base_response()
    };
    let result = instance.handle_error(error, fallback).expect("fallback");
    assert_eq!(result.texts, Some(vec!["fallback".to_owned()]));
}
#[test]
fn generic_guardrail_headers_include_the_api_key_and_json_content_type() {
    let instance = api(
        GenericGuardrailAPISettings::from_optional(None),
        "http://example",
    );
    let headers = instance.prepare_headers().expect("headers");
    let api_key_header = HeaderName::from_static("x-api-key");
    assert_eq!(
        headers.get(&api_key_header).unwrap().to_str().unwrap(),
        "secret-token"
    );
    assert!(headers.get(&api_key_header).unwrap().is_sensitive());
    assert_eq!(
        headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json"
    );
}
#[test]
fn generic_guardrail_headers_omit_the_api_key_when_none_is_configured() {
    let instance = GenericGuardrailAPI {
        api_key: None,
        api_base: "http://example".to_owned(),
        settings: GenericGuardrailAPISettings::from_optional(None),
        provider_params: BTreeMap::new(),
        client: reqwest::Client::default(),
        timeout: super::DEFAULT_TIMEOUT,
    };
    let headers = instance.prepare_headers().expect("headers");
    assert!(!headers.contains_key("x-api-key"));
}
#[test]
fn generic_guardrail_stores_the_api_base_verbatim() {
    let instance = api(
        GenericGuardrailAPISettings::from_optional(None),
        "https://example.com/beta/litellm_basic_guardrail_api",
    );
    assert_eq!(
        instance.api_base,
        "https://example.com/beta/litellm_basic_guardrail_api"
    );
}
#[test]
fn generic_guardrail_enforce_response_blocks_only_a_blocked_action() {
    let blocked = GenericGuardrailAPIResponse {
        action: GuardrailResponseAction::Blocked,
        blocked_reason: Some("policy".to_owned()),
        ..base_response()
    };
    let error =
        super::enforce_response(blocked, &base_request(), &base_request()).expect_err("blocked");
    assert!(matches!(
        error,
        GenericGuardrailAPIError::Blocked { reason } if reason == ": policy"
    ));

    let passthrough = GenericGuardrailAPIResponse {
        action: GuardrailResponseAction::None,
        ..base_response()
    };
    let allowed = super::enforce_response(passthrough, &base_request(), &base_request())
        .expect("passthrough action");
    assert_eq!(allowed.action, GuardrailResponseAction::None);
}

#[test]
fn generic_guardrail_enforce_response_falls_back_to_the_original_request_when_fields_are_empty() {
    let original = GenericGuardrailAPIRequest {
        texts: Some(vec!["original".to_owned()]),
        ..base_request()
    };
    let response = GenericGuardrailAPIResponse {
        texts: None,
        ..base_response()
    };
    let result = super::enforce_response(response, &original, &original).expect("not blocked");
    assert_eq!(result.texts, Some(vec!["original".to_owned()]));
}

#[test]
fn generic_guardrail_enforce_response_keeps_a_non_empty_modified_field() {
    let original = GenericGuardrailAPIRequest {
        texts: Some(vec!["original".to_owned()]),
        ..base_request()
    };
    let response = GenericGuardrailAPIResponse {
        texts: Some(vec!["modified".to_owned()]),
        ..base_response()
    };
    let result = super::enforce_response(response, &original, &original).expect("not blocked");
    assert_eq!(result.texts, Some(vec!["modified".to_owned()]));
}

#[test]
fn generic_guardrail_enforce_response_keeps_modified_structured_messages() {
    let original = GenericGuardrailAPIRequest {
        structured_messages: Some(vec![row("original")]),
        ..base_request()
    };
    let shown = GenericGuardrailAPIRequest {
        structured_messages: Some(vec![row("shown")]),
        ..base_request()
    };
    let response = GenericGuardrailAPIResponse {
        structured_messages: Some(vec![row("modified")]),
        ..base_response()
    };
    let result = super::enforce_response(response, &original, &shown).expect("not blocked");
    assert_eq!(result.structured_messages, Some(vec![row("modified")]));
}

#[test]
fn generic_guardrail_enforce_response_clears_structured_messages_left_unmodified() {
    let shown_row = row("shown");
    let original = GenericGuardrailAPIRequest {
        structured_messages: Some(vec![row("original")]),
        ..base_request()
    };
    let shown = GenericGuardrailAPIRequest {
        structured_messages: Some(vec![shown_row.clone()]),
        ..base_request()
    };
    let response = GenericGuardrailAPIResponse {
        structured_messages: Some(vec![shown_row]),
        ..base_response()
    };
    let result = super::enforce_response(response, &original, &shown).expect("not blocked");
    assert_eq!(result.structured_messages, None);
}

async fn call_api_mock(
    status: StatusCode,
    body: String,
) -> Result<GenericGuardrailAPIResponse, GenericGuardrailAPIError> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}{}", listener.local_addr().unwrap(), DEFAULT_PATH);
    let instance = api(GenericGuardrailAPISettings::from_optional(None), &base);
    let answer = move || async move { (status, body) };
    let router = Router::new().route(DEFAULT_PATH, post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    instance.call_api(&base_request()).await
}

#[tokio::test]
async fn generic_guardrail_call_api_returns_the_response_on_success() {
    let response = call_api_mock(StatusCode::OK, "{}".to_owned())
        .await
        .expect("success");
    assert_eq!(response.action, GuardrailResponseAction::None);
}

#[tokio::test]
async fn generic_guardrail_call_api_rejects_a_non_success_status() {
    let result = call_api_mock(StatusCode::INTERNAL_SERVER_ERROR, "oops".to_owned()).await;
    assert!(matches!(
        result,
        Err(GenericGuardrailAPIError::HttpStatus { status })
            if status == StatusCode::INTERNAL_SERVER_ERROR
    ));
}

// Hardcoded independently of `MAX_RESPONSE_BYTES`, so a mutation of that constant can't move this boundary too.
const SPEC_MAX_RESPONSE_BYTES: usize = 0x0010_0000;

#[tokio::test]
async fn generic_guardrail_call_api_allows_a_body_at_the_size_limit() {
    let body = format!("{{}}{}", " ".repeat(SPEC_MAX_RESPONSE_BYTES - 2));
    assert_eq!(body.len(), SPEC_MAX_RESPONSE_BYTES);
    call_api_mock(StatusCode::OK, body)
        .await
        .expect("body at the limit is accepted");
}

#[tokio::test]
async fn generic_guardrail_call_api_rejects_a_body_over_the_size_limit() {
    let body = " ".repeat(SPEC_MAX_RESPONSE_BYTES + 1);
    let result = call_api_mock(StatusCode::OK, body).await;
    assert!(matches!(
        result,
        Err(GenericGuardrailAPIError::ResponseTooLarge { limit }) if limit == SPEC_MAX_RESPONSE_BYTES
    ));
}
