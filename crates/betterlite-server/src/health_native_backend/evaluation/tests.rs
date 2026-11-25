#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_core::decisions::DecisionsEntryError;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_models::decisions::systemone::DecisionsResponse;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::{EvaluationProbe, evaluation_args, evaluation_reply};
use crate::health_native_backend::seams::EvaluationProbeCall as _;
use crate::health_native_backend::tests::support::{
    EvalSetup, MockServer, fixed_time, http_clients, json, params, text,
};
use crate::health_probe_modes::error::HealthProbeError;
use crate::health_probe_modes::evaluation::evaluation_call;

const EVALUATION_ANSWER: &[u8] = br#"{"model":"model-1","answers":{"reachable":{"type":"noul","noul":0.9}},"usage":{"input_tokens":3,"output_tokens":5}}"#;
const REJECTION: &[u8] =
    br#"{"error":{"message":"invalid api key","type":"authentication_error"}}"#;

fn request_body(request: &str) -> Json {
    let body = request
        .split_once("\r\n\r\n")
        .expect("the provider request has a body")
        .1;
    serde_json::from_str(body).expect("the provider request body is JSON")
}

#[test]
fn evaluation_args_maps_every_documented_field() {
    let entries = [
        ("model", text("model-1")),
        ("api_base", text("https://api.typesafe.ai/v1")),
        ("custom_llm_provider", text("typesafe")),
        ("api_key", text("sk-test")),
        ("timeout", json("16")),
        ("extra_headers", json(r#"{"x-team":"health"}"#)),
        ("top_k", json("5")),
    ];
    let call = evaluation_call(&params(&entries), Some("summarize"), "typesafe")
        .expect("the evaluation call builds");
    let args = evaluation_args(&call.params).expect("the evaluation arguments convert");
    assert_eq!(args.model, "model-1");
    assert_eq!(args.state, Some(text("summarize")));
    let Some(Json::Object(questions)) = &args.questions else {
        panic!("the questions convert to a map");
    };
    assert!(questions.contains_key("reachable"));
    assert_eq!(args.safety_identifier, None);
    assert_eq!(args.api_base.as_deref(), Some("https://api.typesafe.ai/v1"));
    assert_eq!(args.custom_llm_provider.as_deref(), Some("typesafe"));
    assert_eq!(args.timeout, Some(Duration::from_secs(16)));
    assert_eq!(
        args.extra_headers,
        Some(BTreeMap::from([(
            String::from("x-team"),
            String::from("health")
        )]))
    );
    assert_eq!(args.kwargs.get("top_k"), Some(&json("5")));
    let key = args.api_key.expect("the api key converts");
    assert_eq!(key.expose_secret(), "sk-test");
}

#[test]
fn evaluation_args_leave_an_absent_timeout_unset() {
    let entries = [("model", text("model-1"))];
    let call =
        evaluation_call(&params(&entries), None, "typesafe").expect("the evaluation call builds");
    let args = evaluation_args(&call.params).expect("the evaluation arguments convert");
    assert_eq!(args.timeout, None);
    assert_eq!(args.extra_headers, None);
}

#[test]
fn evaluation_args_reject_a_negative_timeout() {
    let entries = [("model", text("model-1")), ("timeout", json("-1"))];
    let call =
        evaluation_call(&params(&entries), None, "typesafe").expect("the evaluation call builds");
    let Err(error) = evaluation_args(&call.params) else {
        panic!("a negative timeout cannot become a duration");
    };
    let HealthProbeError::EvaluationPayload { field, .. } = error else {
        panic!("the timeout failure names its field");
    };
    assert_eq!(field, "timeout");
}

#[test]
fn evaluation_reply_carries_the_hidden_headers() {
    let mut hidden = IndexMap::new();
    hidden.insert(
        String::from("headers"),
        Json::Object(IndexMap::from([(
            String::from("x-ms-region"),
            text("westeurope"),
        )])),
    );
    let response = DecisionsResponse {
        model: None,
        answers: IndexMap::new(),
        usage: None,
        hidden_params: hidden,
        extra: IndexMap::new(),
    };
    let reply = evaluation_reply(&response);
    assert_eq!(
        reply.hidden_headers,
        BTreeMap::from([(String::from("x-ms-region"), String::from("westeurope"))])
    );
}

#[test]
fn evaluation_reply_reads_headers_off_the_hidden_params_only() {
    let mut hidden = IndexMap::new();
    hidden.insert(
        String::from("additional_headers"),
        Json::Object(IndexMap::from([(
            String::from("x-litellm-response-cost"),
            text("0.5"),
        )])),
    );
    hidden.insert(String::from("model"), text("typesafe/model-1"));
    let response = DecisionsResponse {
        model: None,
        answers: IndexMap::new(),
        usage: None,
        hidden_params: hidden,
        extra: IndexMap::new(),
    };
    assert!(evaluation_reply(&response).hidden_headers.is_empty());
}

#[tokio::test]
async fn evaluation_probe_posts_the_health_check_through_asystemone() {
    let server = MockServer::start(200, EVALUATION_ANSWER).await;
    let probe = EvaluationProbe {
        clients: http_clients,
        setup: EvalSetup::default(),
        redactor: Arc::new(Redactor::new(16).expect("the test redactor builds")),
        clock: fixed_time,
    };
    let entries = [
        ("model", text("model-1")),
        ("api_base", text(&server.base)),
        ("custom_llm_provider", text("typesafe")),
        ("api_key", text("test-key")),
    ];
    let call =
        evaluation_call(&params(&entries), None, "typesafe").expect("the evaluation call builds");
    let reply = probe.evaluate(call).await.expect("the evaluation answers");
    assert!(reply.hidden_headers.is_empty());
    let sent = server.requests().await;
    let request = sent.first().expect("one provider call was sent");
    let lowered = request.to_lowercase();
    assert!(lowered.starts_with("post /v1/systemone"), "{lowered}");
    assert!(
        lowered.contains("authorization: bearer test-key"),
        "{lowered}"
    );
    let Json::Object(fields) = request_body(request) else {
        panic!("the provider request body is an object");
    };
    assert_eq!(fields.get("model"), Some(&text("model-1")));
    assert_eq!(fields.get("state"), Some(&text("health check")));
    let Some(Json::Object(questions)) = fields.get("questions") else {
        panic!("the provider request carries the questions map");
    };
    assert!(questions.contains_key("reachable"));
}

#[tokio::test]
async fn evaluation_probe_keeps_a_rejected_call_in_the_backend_error() {
    let server = MockServer::start(401, REJECTION).await;
    let probe = EvaluationProbe {
        clients: http_clients,
        setup: EvalSetup::default(),
        redactor: Arc::new(Redactor::new(16).expect("the test redactor builds")),
        clock: fixed_time,
    };
    let entries = [
        ("model", text("model-1")),
        ("api_base", text(&server.base)),
        ("custom_llm_provider", text("typesafe")),
        ("api_key", text("test-key")),
    ];
    let call =
        evaluation_call(&params(&entries), None, "typesafe").expect("the evaluation call builds");
    let error = probe
        .evaluate(call)
        .await
        .expect_err("the provider rejects the call");
    let HealthProbeError::Backend {
        class,
        message,
        source,
    } = error
    else {
        panic!("a provider rejection is a backend failure");
    };
    assert!(matches!(class, ErrorClass::Authentication(_)));
    assert!(
        message.starts_with("litellm.AuthenticationError: "),
        "{message}"
    );
    assert!(
        source
            .downcast_ref::<DecisionsEntryError>()
            .is_some_and(|entry_failure| entry_failure.to_string() == message),
        "the source is the entrypoint error"
    );
}
