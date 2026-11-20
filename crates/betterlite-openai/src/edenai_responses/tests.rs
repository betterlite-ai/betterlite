#![cfg(test)]

use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Value;

use super::{
    PrepareInput, ReplyRequest, SHOULD_FAKE_STREAM, SUPPORTS_NATIVE_WEBSOCKET, prepare,
    transform_response,
};
use crate::edenai_common::{EDENAI_API_BASE, EdenAiEnv, RESPONSE_COST_HEADER};
use crate::openai_base_config::Headers;
use crate::response_events::{RawResponse, ReplyBody};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

fn fixture(redactor: &Redactor) -> (LoggingState, PrePostEnv<'_>) {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap();
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Silent,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:05:00+00:00").unwrap(),
    };
    (state, env)
}

fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn the_responses_flags_are_false() {
    const { assert!(!SHOULD_FAKE_STREAM) };
    const { assert!(!SUPPORTS_NATIVE_WEBSOCKET) };
}

#[test]
fn prepare_uses_the_eden_default_base_without_any_base() {
    let env = EdenAiEnv::default();
    let prepared = prepare(&PrepareInput {
        headers: Headers::new(),
        api_key: Some(&key("eden")),
        model: "m",
        api_base: None,
        env: &env,
    })
    .unwrap();
    assert_eq!(prepared.url, format!("{EDENAI_API_BASE}/responses"));
}

#[test]
fn prepare_trims_the_trailing_slashes_of_an_explicit_base() {
    let env = EdenAiEnv::default();
    let prepared = prepare(&PrepareInput {
        headers: Headers::new(),
        api_key: Some(&key("eden")),
        model: "m",
        api_base: Some("https://proxy.example/v3///"),
        env: &env,
    })
    .unwrap();
    assert_eq!(prepared.url, "https://proxy.example/v3/responses");
}

#[test]
fn prepare_reads_the_base_from_the_environment() {
    let env = EdenAiEnv {
        api_key: None,
        api_base: Some(String::from("https://gateway.example/v3")),
        global_api_key: None,
    };
    let prepared = prepare(&PrepareInput {
        headers: Headers::new(),
        api_key: Some(&key("eden")),
        model: "m",
        api_base: None,
        env: &env,
    })
    .unwrap();
    assert_eq!(prepared.url, "https://gateway.example/v3/responses");
}

#[test]
fn prepare_adds_only_the_authorization_header() {
    let env = EdenAiEnv::default();
    let prepared = prepare(&PrepareInput {
        headers: Headers::new(),
        api_key: Some(&key("eden")),
        model: "m",
        api_base: None,
        env: &env,
    })
    .unwrap();
    assert_eq!(
        prepared.headers,
        headers(&[("Authorization", "Bearer eden")])
    );
}

#[test]
fn prepare_keeps_the_callers_headers_but_replaces_their_authorization() {
    let env = EdenAiEnv::default();
    let prepared = prepare(&PrepareInput {
        headers: headers(&[("Content-Type", "text/plain"), ("x-trace", "1")]),
        api_key: Some(&key("eden")),
        model: "m",
        api_base: None,
        env: &env,
    })
    .unwrap();
    assert_eq!(
        prepared.headers,
        headers(&[
            ("Authorization", "Bearer eden"),
            ("Content-Type", "text/plain"),
            ("x-trace", "1"),
        ])
    );
}

#[test]
fn prepare_prefers_the_explicit_key_over_the_environment() {
    let env = EdenAiEnv {
        api_key: Some(key("environment")),
        api_base: None,
        global_api_key: None,
    };
    let prepared = prepare(&PrepareInput {
        headers: Headers::new(),
        api_key: Some(&key("explicit")),
        model: "m",
        api_base: None,
        env: &env,
    })
    .unwrap();
    assert_eq!(
        prepared.headers.get("Authorization").map(String::as_str),
        Some("Bearer explicit")
    );
}

#[test]
fn prepare_prefers_the_global_key_over_the_environment() {
    let env = EdenAiEnv {
        api_key: Some(key("environment")),
        api_base: None,
        global_api_key: Some(key("global")),
    };
    let prepared = prepare(&PrepareInput {
        headers: Headers::new(),
        api_key: None,
        model: "m",
        api_base: None,
        env: &env,
    })
    .unwrap();
    assert_eq!(
        prepared.headers.get("Authorization").map(String::as_str),
        Some("Bearer global")
    );
}

#[test]
fn prepare_without_a_key_is_the_missing_key_error() {
    let env = EdenAiEnv::default();
    let error = prepare(&PrepareInput {
        headers: Headers::new(),
        api_key: None,
        model: "m",
        api_base: None,
        env: &env,
    })
    .unwrap_err();
    assert_eq!(error.status_code, 401);
    assert_eq!(
        error.to_string(),
        "Missing Eden AI API key: set EDENAI_API_KEY or pass api_key"
    );
}

const BODY: &str = r#"{"id":"resp_1","created_at":1,"output":[],"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3,"cost":0.25}}"#;

fn reply_body<'request>(
    headers: &'request IndexMap<String, String>,
    text: &'request str,
) -> RawResponse<'request> {
    RawResponse {
        status: 200,
        headers,
        text,
    }
}

#[test]
fn transform_response_lifts_the_usage_cost_into_the_hidden_headers() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = fixture(&redactor);
    let reply_headers = IndexMap::from([(String::from("x-request-id"), String::from("req-1"))]);
    let reply = reply_body(&reply_headers, BODY);
    let transformed = transform_response(ReplyRequest {
        reply: &reply,
        now: 7,
        state: &mut state,
        env: &env,
    })
    .unwrap();
    assert!(matches!(&transformed.response, ReplyBody::Typed(_)));
    let additional = serde_json::to_value(&transformed.additional_headers).unwrap();
    assert_eq!(additional[RESPONSE_COST_HEADER], Value::from(0.25_f64));
    assert_eq!(
        additional["llm_provider-x-request-id"],
        Value::from("req-1")
    );
}

#[test]
fn transform_response_without_a_cost_keeps_the_reply_headers() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = fixture(&redactor);
    let reply_headers = IndexMap::from([(String::from("x-request-id"), String::from("req-1"))]);
    let body = r#"{"id":"resp_1","created_at":1,"output":[]}"#;
    let reply = reply_body(&reply_headers, body);
    let transformed = transform_response(ReplyRequest {
        reply: &reply,
        now: 7,
        state: &mut state,
        env: &env,
    })
    .unwrap();
    let additional = serde_json::to_value(&transformed.additional_headers).unwrap();
    assert_eq!(additional[RESPONSE_COST_HEADER], Value::Null);
    assert_eq!(
        additional["llm_provider-x-request-id"],
        Value::from("req-1")
    );
}

#[test]
fn transform_response_without_a_usage_has_no_cost() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = fixture(&redactor);
    let reply_headers = IndexMap::new();
    let body = r#"{"id":"resp_1","created_at":1,"output":[],"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}"#;
    let reply = reply_body(&reply_headers, body);
    let transformed = transform_response(ReplyRequest {
        reply: &reply,
        now: 7,
        state: &mut state,
        env: &env,
    })
    .unwrap();
    let additional = serde_json::to_value(&transformed.additional_headers).unwrap();
    assert_eq!(additional[RESPONSE_COST_HEADER], Value::Null);
}
