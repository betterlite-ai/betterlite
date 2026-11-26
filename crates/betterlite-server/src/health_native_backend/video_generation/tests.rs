#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_core::api::video_records::VideoObject;
use betterlite_core::api::video_remix_call::config::DEFAULT_TIMEOUT_SECONDS;
use betterlite_core::api::videos_http_transport::video_config::{Headers, Params};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Number;

use super::{generation_reply, generation_request, restore_logging, take_logging};
use crate::health_probe_modes::calls::ProbeCall;
use crate::health_probe_modes::error::HealthProbeError;
use crate::health_probe_modes::logging::{HealthProbeLogging, ProbeLogging};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn probe(parameters: &[(&str, Json)]) -> ProbeCall {
    ProbeCall {
        parameters: parameters
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
        api_key: None,
        logging: None,
    }
}

fn header(name: &str, value: &str) -> IndexMap<String, Json> {
    IndexMap::from([(name.to_owned(), text(value))])
}

fn fixed_time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-02T03:04:05+00:00").expect("the test time parses")
}

fn state(model: &str) -> LoggingState {
    let mut correlation = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut correlation,
        settings: CorrelationSettings::default(),
    };
    let init = LoggingInit {
        model: model.to_owned(),
        call_type: String::new(),
        start_time: fixed_time(),
        litellm_call_id: String::new(),
        function_id: String::new(),
        messages: None,
        stream: None,
        litellm_trace_id: None,
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    let redactor = Redactor::new(16).expect("the test redactor builds");
    LoggingState::new(init, setup, &redactor, String::new, || Duration::ZERO)
        .expect("the test logging state builds")
}

#[test]
fn generation_request_binds_the_named_text_arguments() {
    let call = probe(&[
        ("prompt", text("a prompt")),
        ("model", text("sora-2")),
        ("custom_llm_provider", text("openai")),
        ("seconds", text("8")),
        ("size", text("720x1280")),
        ("user", text("user-1")),
    ]);
    let request = generation_request(&call).expect("the health arguments convert");
    assert_eq!(request.prompt, "a prompt");
    assert_eq!(request.model.as_deref(), Some("sora-2"));
    assert_eq!(request.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(request.seconds.as_deref(), Some("8"));
    assert_eq!(request.size.as_deref(), Some("720x1280"));
    assert_eq!(request.user.as_deref(), Some("user-1"));
}

#[test]
fn generation_request_carries_the_extra_maps() {
    let call = probe(&[
        ("prompt", text("a prompt")),
        ("extra_headers", Json::Object(header("x-team", "health"))),
        ("extra_query", Json::Object(header("cursor", "after"))),
        ("extra_body", Json::Object(header("size", "480x854"))),
    ]);
    let request = generation_request(&call).expect("the health arguments convert");
    assert_eq!(
        request.extra_headers,
        Some(Headers::from([("x-team".to_owned(), "health".to_owned())]))
    );
    assert_eq!(
        request.extra_query,
        Some(Params::from([("cursor".to_owned(), text("after"))]))
    );
    assert_eq!(
        request.extra_body,
        Some(Params::from([("size".to_owned(), text("480x854"))]))
    );
}

#[test]
fn generation_request_keeps_the_loose_keywords() {
    let mut call = probe(&[
        ("prompt", text("a prompt")),
        ("model", text("sora-2")),
        ("extra_headers", Json::Object(header("x-team", "health"))),
        ("api_base", text("https://api.openai.com/v1")),
        ("resolution", text("1080x1920")),
    ]);
    call.api_key = Some(SecretString::from("sk-test".to_owned()));
    let request = generation_request(&call).expect("the health arguments convert");
    assert_eq!(
        request.kwargs.get("api_base"),
        Some(&text("https://api.openai.com/v1"))
    );
    assert_eq!(request.kwargs.get("resolution"), Some(&text("1080x1920")));
    assert_eq!(request.kwargs.get("api_key"), Some(&text("sk-test")));
    assert!(!request.kwargs.contains_key("prompt"));
    assert!(!request.kwargs.contains_key("model"));
    assert!(!request.kwargs.contains_key("extra_headers"));
    assert!(!request.kwargs.contains_key("timeout"));
}

#[test]
fn generation_request_keeps_the_signature_timeout() {
    let absent = probe(&[("prompt", text("a prompt"))]);
    let request = generation_request(&absent).expect("the health arguments convert");
    assert_eq!(request.timeout, Some(DEFAULT_TIMEOUT_SECONDS));
    let null = probe(&[("prompt", text("a prompt")), ("timeout", Json::Null)]);
    let request = generation_request(&null).expect("the health arguments convert");
    assert_eq!(request.timeout, None);
}

#[test]
fn generation_request_takes_a_numeric_timeout() {
    let call = probe(&[
        ("prompt", text("a prompt")),
        ("timeout", Json::Number(Number::from(30_u64))),
    ]);
    let request = generation_request(&call).expect("the health arguments convert");
    assert_eq!(request.timeout, Some(30.0_f64));
}

#[test]
fn generation_request_refuses_a_parameter_of_the_wrong_type() {
    let call = probe(&[
        ("prompt", text("a prompt")),
        ("seconds", Json::Number(Number::from(8_u64))),
    ]);
    let Err(error) = generation_request(&call) else {
        panic!("a number cannot be seconds");
    };
    let HealthProbeError::ParameterValue { key, .. } = error else {
        panic!("the failure names the parameter it could not convert");
    };
    assert_eq!(key, "seconds");
}

#[test]
fn generation_request_requires_the_prompt() {
    let call = probe(&[("model", text("sora-2"))]);
    let Err(error) = generation_request(&call) else {
        panic!("a prompt is required");
    };
    let HealthProbeError::ParameterValue { key, .. } = error else {
        panic!("the failure names the parameter it could not convert");
    };
    assert_eq!(key, "prompt");
}

#[test]
fn generation_reply_carries_the_hidden_headers() {
    let mut object = VideoObject::new("video-1".to_owned(), "completed".to_owned());
    object.hidden_params.insert(
        "headers".to_owned(),
        Json::Object(header("x-ms-region", "westeurope")),
    );
    let reply = generation_reply(&object);
    assert_eq!(
        reply.hidden_headers,
        BTreeMap::from([(String::from("x-ms-region"), String::from("westeurope"))])
    );
}

#[test]
fn the_call_swaps_the_shared_logging_state_out_and_back() {
    let logging: Arc<dyn HealthProbeLogging> = Arc::new(ProbeLogging::from(state("stored")));
    let shared = logging
        .logging_state()
        .expect("the health object exposes its state");
    let mut call = probe(&[("prompt", text("a prompt"))]);
    call.logging = Some(logging);
    let (taken, restored) = take_logging(&call, state("placeholder"));
    assert_eq!(taken.model, "stored");
    assert_eq!(restored, Some(Arc::clone(&shared)));
    let current = shared.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(current.model, "placeholder");
    drop(current);
    restore_logging(restored, state("updated"));
    let current = shared.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(current.model, "updated");
}

#[test]
fn a_call_without_a_logging_object_takes_the_placeholder() {
    let call = probe(&[("prompt", text("a prompt"))]);
    let (taken, restored) = take_logging(&call, state("placeholder"));
    assert_eq!(taken.model, "placeholder");
    assert_eq!(restored, None);
}
