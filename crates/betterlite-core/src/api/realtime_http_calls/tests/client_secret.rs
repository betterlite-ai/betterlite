use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionError;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};

use super::super::DEFAULT_REALTIME_MODEL;
use super::super::call::SuppliedClient;
use super::super::dispatch::SessionEndpoint;
use super::super::error::{RealtimeHttpCallsError, RecordError};
use super::{
    EMPTY_SETTINGS, FakeFailure, GLOBALS, Harness, OPENAI_MODEL, ProviderTag, Seen,
    client_secret_call, fields, json, no_kwargs, only_session, secret,
};
use crate::api::vector_store_http_write::service::ClientKind;

#[test]
fn client_secret_takes_raw_credentials_over_every_fallback() {
    let harness = Harness::new(
        GLOBALS,
        &[
            ("OPENAI_API_BASE", "https://env.example/v1"),
            ("OPENAI_API_KEY", "sk-env"),
        ],
    );
    let kwargs = no_kwargs();
    let key = secret("sk-raw");
    let mut call = client_secret_call(&kwargs, Some(OPENAI_MODEL));
    call.common.api_base = Some("https://raw.example/v1/");
    call.common.api_key = Some(&key);
    harness.run_client_secret(call).expect("the client secret");
    let seen = harness.seen();
    let posting = only_session(&seen);
    assert_eq!(posting.endpoint, SessionEndpoint::ClientSecrets);
    assert_eq!(posting.provider, ProviderTag::OpenAi);
    assert_eq!(posting.model, "gpt-4o-realtime-preview");
    assert_eq!(posting.api_base, "https://raw.example/v1");
    assert_eq!(posting.api_key, "sk-raw");
    assert!(posting.request_data.is_empty());
    let logging = harness
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(logging.model, "gpt-4o-realtime-preview");
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        logging.litellm_params,
        fields(r#"{"api_base":"https://raw.example/v1"}"#)
    );
    let encoded =
        serde_json::to_string(&logging.model_call_details).expect("the details serialize");
    assert!(!encoded.contains("sk-raw"));
}

#[test]
fn client_secret_takes_the_litellm_globals_over_the_environment() {
    let harness = Harness::new(
        GLOBALS,
        &[
            ("OPENAI_API_BASE", "https://env.example/v1"),
            ("OPENAI_API_KEY", "sk-env"),
        ],
    );
    let kwargs = no_kwargs();
    let call = client_secret_call(&kwargs, Some(OPENAI_MODEL));
    harness.run_client_secret(call).expect("the client secret");
    let seen = harness.seen();
    let posting = only_session(&seen);
    assert_eq!(posting.api_base, "https://global.example/v1");
    assert_eq!(posting.api_key, "sk-global");
}

#[test]
fn client_secret_falls_back_to_openai_key_then_the_environment_then_defaults() {
    let environment = [
        ("OPENAI_API_BASE", "https://env.example/v1"),
        ("OPENAI_API_KEY", "sk-env"),
    ];
    let openai_key = "litellm_settings:\n  openai_key: sk-openai-global\n";
    let with_openai_key = Harness::new(openai_key, &environment);
    let kwargs = no_kwargs();
    with_openai_key
        .run_client_secret(client_secret_call(&kwargs, Some(OPENAI_MODEL)))
        .expect("the openai_key call");
    let first = with_openai_key.seen();
    let openai_key_posting = only_session(&first);
    assert_eq!(openai_key_posting.api_base, "https://env.example/v1");
    assert_eq!(openai_key_posting.api_key, "sk-openai-global");
    let from_environment = Harness::new(EMPTY_SETTINGS, &environment);
    from_environment
        .run_client_secret(client_secret_call(&kwargs, Some(OPENAI_MODEL)))
        .expect("the environment call");
    let second = from_environment.seen();
    let environment_posting = only_session(&second);
    assert_eq!(environment_posting.api_base, "https://env.example/v1");
    assert_eq!(environment_posting.api_key, "sk-env");
    let without_anything = Harness::new(EMPTY_SETTINGS, &[]);
    without_anything
        .run_client_secret(client_secret_call(&kwargs, Some(OPENAI_MODEL)))
        .expect("the default call");
    let third = without_anything.seen();
    let default_posting = only_session(&third);
    assert_eq!(default_posting.api_base, "https://api.openai.com");
    assert_eq!(default_posting.api_key, "");
}

#[test]
fn client_secret_leaves_a_known_provider_without_a_config_on_the_fallback_chain() {
    let harness = Harness::new(GLOBALS, &[]);
    let kwargs = no_kwargs();
    let mut call = client_secret_call(&kwargs, Some("anthropic/claude-3-5-sonnet-latest"));
    call.common.api_base = Some("https://fallback.example/v1");
    harness.run_client_secret(call).expect("the fallback call");
    let seen = harness.seen();
    let posting = only_session(&seen);
    assert_eq!(posting.provider, ProviderTag::None);
    assert_eq!(posting.api_base, "https://fallback.example/v1");
    assert_eq!(posting.api_key, "sk-openai-global");
}

#[test]
fn client_secret_rejects_the_default_model_without_a_request() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let error = harness
        .run_client_secret(client_secret_call(&kwargs, None))
        .expect_err("the bare default model resolves no provider");
    let RealtimeHttpCallsError::Resolution(ResolutionError::MissingProvider { model }) = error
    else {
        panic!("the default model is unresolvable");
    };
    assert_eq!(model, DEFAULT_REALTIME_MODEL);
    assert!(harness.seen().is_empty());
}

#[test]
fn client_secret_routes_on_the_session_model_and_rewrites_it() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let session = json(r#"{"model":"openai/gpt-4o-realtime-preview","type":"realtime"}"#);
    let mut call = client_secret_call(&kwargs, Some("azure/wrong-route"));
    call.session = Some(&session);
    harness.run_client_secret(call).expect("the client secret");
    let seen = harness.seen();
    let posting = only_session(&seen);
    assert_eq!(posting.provider, ProviderTag::OpenAi);
    assert_eq!(posting.model, "gpt-4o-realtime-preview");
    assert!(!posting.request_data.contains_key("model"));
    assert_eq!(
        posting.request_data,
        fields(r#"{"session":{"type":"realtime","model":"gpt-4o-realtime-preview"}}"#)
    );
    let logging = harness
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(logging.model, "gpt-4o-realtime-preview");
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("openai"));
}

#[test]
fn client_secret_omits_an_empty_session_and_expiry() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let session = json("{}");
    let expires_after = json("{}");
    let mut call = client_secret_call(&kwargs, Some(OPENAI_MODEL));
    call.session = Some(&session);
    call.expires_after = Some(&expires_after);
    harness.run_client_secret(call).expect("the client secret");
    let seen = harness.seen();
    let posting = only_session(&seen);
    assert!(posting.request_data.is_empty());
    let logging = harness
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        logging.optional_params,
        fields(r#"{"expires_after":{},"session":{}}"#)
    );
}

#[test]
fn client_secret_fills_the_expiry_anchor() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let seconds = json(r#"{"seconds":60}"#);
    let mut seconds_call = client_secret_call(&kwargs, Some(OPENAI_MODEL));
    seconds_call.expires_after = Some(&seconds);
    harness
        .run_client_secret(seconds_call)
        .expect("the expiry call");
    let zero = json(r#"{"anchor":"created_at","seconds":0}"#);
    let mut zero_call = client_secret_call(&kwargs, Some(OPENAI_MODEL));
    zero_call.expires_after = Some(&zero);
    harness.run_client_secret(zero_call).expect("the zero call");
    let null_anchor = json(r#"{"anchor":null,"seconds":5}"#);
    let mut null_anchor_call = client_secret_call(&kwargs, Some(OPENAI_MODEL));
    null_anchor_call.expires_after = Some(&null_anchor);
    harness
        .run_client_secret(null_anchor_call)
        .expect("the null anchor call");
    let seen = harness.seen();
    assert_eq!(seen.len(), 3);
    let first = expiry(&seen, 0);
    let first_expected = fields(r#"{"anchor":"created_at","seconds":60}"#);
    assert_eq!(first, Some(&Json::Object(first_expected)));
    let second = expiry(&seen, 1);
    let second_expected = fields(r#"{"anchor":"created_at","seconds":0}"#);
    assert_eq!(second, Some(&Json::Object(second_expected)));
    let third = expiry(&seen, 2);
    let third_expected = fields(r#"{"seconds":5}"#);
    assert_eq!(third, Some(&Json::Object(third_expected)));
}

fn expiry(seen: &[Seen], index: usize) -> Option<&Json> {
    let Seen::Session(posting) = &seen[index] else {
        panic!("the request is a session POST");
    };
    posting.request_data.get("expires_after")
}

#[test]
fn client_secret_drops_null_fields_and_keeps_unknown_session_fields() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let session = json(
        r#"{"type":"realtime","instructions":null,
            "audio":{"input":{"format":"pcm16"},"nested":null},
            "vendor_extension":{"flag":true}}"#,
    );
    let mut call = client_secret_call(&kwargs, Some(OPENAI_MODEL));
    call.session = Some(&session);
    harness.run_client_secret(call).expect("the client secret");
    let seen = harness.seen();
    let posting = only_session(&seen);
    assert_eq!(
        posting.request_data,
        fields(
            r#"{"session":{"type":"realtime",
                "audio":{"input":{"format":"pcm16"},"nested":null},
                "vendor_extension":{"flag":true}}}"#
        )
    );
}

#[test]
fn client_secret_leaves_a_null_session_model_out_of_the_body() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let session = json(r#"{"model":null,"type":"realtime"}"#);
    let mut call = client_secret_call(&kwargs, Some(OPENAI_MODEL));
    call.session = Some(&session);
    harness.run_client_secret(call).expect("the client secret");
    let seen = harness.seen();
    let posting = only_session(&seen);
    assert_eq!(posting.model, "gpt-4o-realtime-preview");
    assert_eq!(
        posting.request_data,
        fields(r#"{"session":{"type":"realtime"}}"#)
    );
}

#[test]
fn client_secret_returns_the_dispatched_response_unchanged() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    harness
        .recorder
        .reply(201, br#"{"value":"ek_1","expires_at":1700000000}"#, headers);
    let kwargs = no_kwargs();
    let response = harness
        .run_client_secret(client_secret_call(&kwargs, Some(OPENAI_MODEL)))
        .expect("the client secret");
    assert_eq!(response.status, 201);
    assert_eq!(
        response.body,
        br#"{"value":"ek_1","expires_at":1700000000}"#.to_vec()
    );
    assert_eq!(
        response
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
}

#[test]
fn client_secret_forwards_headers_api_version_and_the_callers_client() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let environment = BTreeMap::new();
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &environment).expect("the proxy policy"),
    );
    let client = ProxiedClient::outbound(&policy).expect("the proxied client");
    let extra_headers = BTreeMap::from([
        (String::from("x-extra"), String::from("1")),
        (String::from("x-second"), String::from("2")),
    ]);
    let kwargs = no_kwargs();
    let mut call = client_secret_call(&kwargs, Some(OPENAI_MODEL));
    call.common.extra_headers = Some(&extra_headers);
    call.common.api_version = Some("2024-10-01");
    call.common.client = Some(SuppliedClient {
        kind: ClientKind::Sync,
        transport: &client,
    });
    harness.run_client_secret(call).expect("the client secret");
    let seen = harness.seen();
    let posting = only_session(&seen);
    assert_eq!(posting.extra_headers, Some(extra_headers));
    assert_eq!(posting.api_version.as_deref(), Some("2024-10-01"));
    assert_eq!(posting.client_kind, Some(ClientKind::Sync));
}

#[test]
fn client_secret_rejects_a_non_object_session_before_any_request() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let session = json(r#""not-a-session""#);
    let mut call = client_secret_call(&kwargs, Some(OPENAI_MODEL));
    call.session = Some(&session);
    let error = harness
        .run_client_secret(call)
        .expect_err("a string is not a session");
    let RealtimeHttpCallsError::Record(RecordError::Invalid { field, .. }) = error else {
        panic!("the session record is invalid");
    };
    assert_eq!(field, "session");
    assert!(harness.seen().is_empty());
}

#[test]
fn client_secret_propagates_the_dispatch_failure() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    harness.recorder.fail_once("boom");
    let kwargs = no_kwargs();
    let error = harness
        .run_client_secret(client_secret_call(&kwargs, Some(OPENAI_MODEL)))
        .expect_err("the transport fails");
    let RealtimeHttpCallsError::Dispatch(failure) = error else {
        panic!("the transport failure is kept");
    };
    assert_eq!(failure, FakeFailure { message: "boom" });
    assert_eq!(harness.seen().len(), 1);
}

#[test]
fn client_secret_logs_the_raw_optional_params_and_the_resolved_api_base() {
    let harness = Harness::new(EMPTY_SETTINGS, &[]);
    let kwargs = no_kwargs();
    let session = json(r#"{"model":"openai/gpt-4o-realtime-preview","instructions":"hi"}"#);
    let expires_after = json(r#"{"seconds":60}"#);
    let mut call = client_secret_call(&kwargs, None);
    call.session = Some(&session);
    call.expires_after = Some(&expires_after);
    call.common.api_base = Some("https://raw.example/v1");
    harness.run_client_secret(call).expect("the client secret");
    let logging = harness
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        logging.optional_params,
        fields(
            r#"{"expires_after":{"seconds":60},
                "session":{"model":"openai/gpt-4o-realtime-preview","instructions":"hi"}}"#
        )
    );
    assert_eq!(
        logging.litellm_params,
        fields(r#"{"api_base":"https://raw.example/v1"}"#)
    );
    assert_eq!(logging.model, "gpt-4o-realtime-preview");
}
