#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::enkryptai_detection::{Configured, DEFAULT_API_BASE};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_http::proxy::base_builder;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::EnkryptAiGuardrail;
use super::logging::DiscardSpans;
use super::process::{Processed, process};
use super::wire::{DetectPayload, DetectionResponse};

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;

struct Reply {
    status: StatusCode,
    body: String,
}

struct Captured {
    path: String,
    headers: HeaderMap,
    body: String,
}

#[derive(Clone)]
struct TestState {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    captured: Arc<Mutex<Vec<Captured>>>,
}

async fn handle(
    State(state): State<TestState>,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> (StatusCode, String) {
    state.captured.lock().unwrap().push(Captured {
        path: uri.path().to_owned(),
        headers,
        body,
    });
    match state.replies.lock().unwrap().pop_front() {
        Some(reply) => (reply.status, reply.body),
        None => (StatusCode::INTERNAL_SERVER_ERROR, "{}".to_owned()),
    }
}

async fn serve(replies: Vec<Reply>) -> (String, TestState, JoinSet<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let state = TestState {
        replies: Arc::new(Mutex::new(VecDeque::from(replies))),
        captured: Arc::new(Mutex::new(Vec::new())),
    };
    let router = Router::new().fallback(handle).with_state(state.clone());
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, state, server)
}

fn clock() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn settings_for(api_base: &str) -> Configured {
    Configured {
        guardrail_name: "guardrail".to_owned(),
        api_key: SecretString::from("secret-key"),
        api_base: api_base.to_owned(),
        policy_name: None,
        deployment_name: None,
        detectors: None,
        block_on_violation: true,
        mode: None,
        timeout: Duration::from_secs(5),
    }
}

fn guardrail_with(settings: Configured) -> EnkryptAiGuardrail {
    EnkryptAiGuardrail::new(
        settings,
        base_builder().build().expect("client"),
        clock,
        Arc::new(DiscardSpans),
        true,
    )
    .expect("guardrail")
}

fn guardrail(api_base: &str) -> EnkryptAiGuardrail {
    let settings = settings_for(api_base);
    guardrail_with(settings)
}

fn inputs(texts: &[&str]) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

fn payload(captured: &Captured) -> DetectPayload {
    serde_json::from_str(&captured.body).expect("payload")
}

fn record_objects(request_data: &BTreeMap<String, GuardrailValue>) -> Vec<GuardrailValue> {
    let Some(GuardrailValue::Object(metadata)) = request_data.get("metadata") else {
        return Vec::new();
    };
    let Some(GuardrailValue::Array(records)) =
        metadata.get("standard_logging_guardrail_information")
    else {
        return Vec::new();
    };
    records.clone()
}

fn field(value: &GuardrailValue, name: &str) -> GuardrailValue {
    let GuardrailValue::Object(fields) = value else {
        panic!("record is not an object: {value:?}");
    };
    fields
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("missing field {name}"))
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn parse_value(body: &str) -> GuardrailValue {
    serde_json::from_str(body).expect("json value")
}

fn parsed_response(raw: &GuardrailValue) -> DetectionResponse {
    let value = serde_json::to_value(raw).expect("value");
    serde_json::from_value(value).expect("response")
}

#[tokio::test]
async fn enkryptai_guardrail_sends_the_detection_request() {
    let (base, state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body: r#"{"summary":{},"details":{}}"#.to_owned(),
    }])
    .await;
    let mut request_data = BTreeMap::new();
    let returned = guardrail(&base)
        .apply(inputs(&["hello"]), &mut request_data, None)
        .await
        .expect("apply");
    assert_eq!(returned.texts, Some(vec!["hello".to_owned()]));
    let captured = state.captured.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].path, "/guardrails/policy/detect");
    assert_eq!(payload(&captured[0]).text, "hello");
    assert_eq!(captured[0].headers["content-type"], "application/json");
    assert_eq!(captured[0].headers["apikey"], "secret-key");
    assert!(!captured[0].headers.contains_key("x-enkrypt-policy"));
    drop(captured);
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_sends_the_policy_header_when_configured() {
    let (base, state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body: r#"{"summary":{},"details":{}}"#.to_owned(),
    }])
    .await;
    let mut settings = settings_for(&base);
    settings.policy_name = Some("policy-a".to_owned());
    guardrail_with(settings)
        .apply(inputs(&["hello"]), &mut BTreeMap::new(), None)
        .await
        .expect("apply");
    let captured = state.captured.lock().unwrap();
    assert_eq!(captured[0].headers["x-enkrypt-policy"], "policy-a");
    drop(captured);
    server.shutdown().await;
}

#[test]
fn enkryptai_guardrail_supports_the_litellm_event_hooks() {
    let guardrail = guardrail("http://127.0.0.1:1");
    assert_eq!(
        guardrail.base.settings.supported_event_hooks,
        Some(super::SUPPORTED_EVENT_HOOKS.to_vec())
    );
}

#[tokio::test]
async fn enkryptai_guardrail_preserves_the_configured_api_base() {
    let (base, state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body: r#"{"summary":{},"details":{}}"#.to_owned(),
    }])
    .await;
    let mut settings = settings_for(&base);
    settings.api_base = format!("{base}/");
    guardrail_with(settings)
        .apply(inputs(&["hello"]), &mut BTreeMap::new(), None)
        .await
        .expect("apply");
    let captured = state.captured.lock().unwrap();
    assert_eq!(captured[0].path, "//guardrails/policy/detect");
    drop(captured);
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_records_success_information() {
    let (base, _state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body: r#"{"summary":{"toxicity":[]},"details":{}}"#.to_owned(),
    }])
    .await;
    let mut settings = settings_for(&base);
    settings.mode = Some(LitellmParamsMode::One("post_call".to_owned()));
    let mut request_data = BTreeMap::new();
    guardrail_with(settings)
        .apply(inputs(&["hello"]), &mut request_data, None)
        .await
        .expect("apply");
    let records = record_objects(&request_data);
    assert_eq!(records.len(), 1);
    assert_eq!(field(&records[0], "guardrail_mode"), text("post_call"));
    assert_eq!(field(&records[0], "guardrail_provider"), text("enkryptai"));
    assert_eq!(field(&records[0], "guardrail_status"), text("success"));
    assert_eq!(
        field(&records[0], "guardrail_response"),
        parse_value(r#"{"summary":{"toxicity":[]},"details":{}}"#)
    );
    assert_eq!(
        field(&records[0], "start_time"),
        GuardrailValue::Decimal(1_700_000_000.0)
    );
    assert_eq!(field(&records[0], "duration"), GuardrailValue::Decimal(0.0));
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_detects_policy_violation() {
    let (base, _state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body: r#"{"summary":{"policy_violation":1},"details":{"policy_violation":{"violating_policy":"no-illegal","explanation":"illegal content"}}}"#.to_owned(),
    }])
    .await;
    let mut request_data = BTreeMap::new();
    let error = guardrail(&base)
        .apply(inputs(&["bad"]), &mut request_data, None)
        .await
        .expect_err("detected");
    let message = match error {
        GuardrailFailure::GuardrailRaised {
            message,
            status_code: 400,
            ..
        } => message,
        other => panic!("unexpected failure: {other:?}"),
    };
    assert_eq!(
        message,
        "EnkryptAI guardrail detected 1 violation(s):\n- POLICY_VIOLATION: policy \"no-illegal\", explanation \"illegal content\""
    );
    let records = record_objects(&request_data);
    assert_eq!(
        field(&records[0], "guardrail_status"),
        text("guardrail_intervened")
    );
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_defaults_null_attack_details() {
    let (base, _state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body: r#"{"summary":{"policy_violation":1},"details":{"policy_violation":null}}"#
            .to_owned(),
    }])
    .await;
    let mut request_data = BTreeMap::new();
    let error = guardrail(&base)
        .apply(inputs(&["bad"]), &mut request_data, None)
        .await
        .expect_err("detected");
    let message = match error {
        GuardrailFailure::GuardrailRaised {
            message,
            status_code: 400,
            ..
        } => message,
        other => panic!("unexpected failure: {other:?}"),
    };
    assert_eq!(
        message,
        "EnkryptAI guardrail detected 1 violation(s):\n- POLICY_VIOLATION: policy \"N/A\", explanation \"N/A\""
    );
    let records = record_objects(&request_data);
    assert_eq!(
        field(&records[0], "guardrail_status"),
        text("guardrail_intervened")
    );
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_detects_pii() {
    let (base, _state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body: r#"{"summary":{"pii":1},"details":{"pii":{"pii":{"email":1,"phone":2}}}}"#.to_owned(),
    }])
    .await;
    let error = guardrail(&base)
        .apply(inputs(&["bad"]), &mut BTreeMap::new(), None)
        .await
        .expect_err("detected");
    let message = match error {
        GuardrailFailure::GuardrailRaised {
            message,
            status_code: 400,
            ..
        } => message,
        other => panic!("unexpected failure: {other:?}"),
    };
    assert_eq!(
        message,
        "EnkryptAI guardrail detected 1 violation(s):\n- PII: pii detected: {\"email\":1,\"phone\":2}"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_reports_only_toxicity_scores_above_half() {
    let (base, _state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body: r#"{"summary":{"toxicity":["toxic","insult"]},"details":{"toxicity":{"toxic":0.9,"threat":0.5,"obscene":0.4,"insult":0.51}}}"#.to_owned(),
    }])
    .await;
    let error = guardrail(&base)
        .apply(inputs(&["bad"]), &mut BTreeMap::new(), None)
        .await
        .expect_err("detected");
    let message = match error {
        GuardrailFailure::GuardrailRaised {
            message,
            status_code: 400,
            ..
        } => message,
        other => panic!("unexpected failure: {other:?}"),
    };
    assert_eq!(
        message,
        "EnkryptAI guardrail detected 1 violation(s):\n- TOXICITY: toxicity scores above 0.5: toxic, insult"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_detects_keywords_and_bias() {
    let (base, _state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body: r#"{"summary":{"keyword_detected":1,"bias":1},"details":{"keyword_detected":{"detected_keywords":["secret","weapon"]},"bias":{"bias_detected":true}}}"#.to_owned(),
    }])
    .await;
    let error = guardrail(&base)
        .apply(inputs(&["bad"]), &mut BTreeMap::new(), None)
        .await
        .expect_err("detected");
    let message = match error {
        GuardrailFailure::GuardrailRaised {
            message,
            status_code: 400,
            ..
        } => message,
        other => panic!("unexpected failure: {other:?}"),
    };
    assert_eq!(
        message,
        "EnkryptAI guardrail detected 2 violation(s):\n- KEYWORD_DETECTED: detected keywords: [\"secret\",\"weapon\"]\n- BIAS: bias detected: true"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_detects_prompt_injection_and_jailbreak() {
    let (base, _state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body: r#"{"summary":{"prompt_injection":1,"jailbreak":1},"details":{"prompt_injection":{"score":0.8},"jailbreak":{"score":0.9}}}"#.to_owned(),
    }])
    .await;
    let error = guardrail(&base)
        .apply(inputs(&["bad"]), &mut BTreeMap::new(), None)
        .await
        .expect_err("detected");
    let message = match error {
        GuardrailFailure::GuardrailRaised {
            message,
            status_code: 400,
            ..
        } => message,
        other => panic!("unexpected failure: {other:?}"),
    };
    assert_eq!(
        message,
        "EnkryptAI guardrail detected 2 violation(s):\n- PROMPT_INJECTION: details: {\"score\":0.8}\n- JAILBREAK: details: {\"score\":0.9}"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_reports_unknown_detector_details() {
    let (base, _state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body: r#"{"summary":{"nsfw":1,"spam":1},"details":{"nsfw":{"score":0.7}}}"#.to_owned(),
    }])
    .await;
    let error = guardrail(&base)
        .apply(inputs(&["bad"]), &mut BTreeMap::new(), None)
        .await
        .expect_err("detected");
    let message = match error {
        GuardrailFailure::GuardrailRaised {
            message,
            status_code: 400,
            ..
        } => message,
        other => panic!("unexpected failure: {other:?}"),
    };
    assert_eq!(
        message,
        "EnkryptAI guardrail detected 2 violation(s):\n- NSFW: details: {\"score\":0.7}\n- SPAM: details: {}"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_batch_evaluates_each_text() {
    let (base, state, mut server) = serve(vec![
        Reply {
            status: StatusCode::OK,
            body: r#"{"summary":{},"details":{}}"#.to_owned(),
        },
        Reply {
            status: StatusCode::OK,
            body: r#"{"summary":{"bias":0},"details":{}}"#.to_owned(),
        },
    ])
    .await;
    let mut request_data = BTreeMap::new();
    let returned = guardrail(&base)
        .apply(inputs(&["first", "second"]), &mut request_data, None)
        .await
        .expect("apply");
    assert_eq!(
        returned.texts,
        Some(vec!["first".to_owned(), "second".to_owned()])
    );
    let captured = state.captured.lock().unwrap();
    assert_eq!(captured.len(), 2);
    assert_eq!(payload(&captured[1]).text, "second");
    drop(captured);
    let records = record_objects(&request_data);
    assert_eq!(records.len(), 2);
    assert_eq!(field(&records[1], "guardrail_status"), text("success"));
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_batch_stops_at_the_first_detected_text() {
    let replies = [
        r#"{"summary":{},"details":{}}"#,
        r#"{"summary":{"jailbreak":1},"details":{"jailbreak":{"score":0.9}}}"#,
        r#"{"summary":{},"details":{}}"#,
    ]
    .map(|body| Reply {
        status: StatusCode::OK,
        body: body.to_owned(),
    });
    let (base, state, mut server) = serve(Vec::from(replies)).await;
    let mut request_data = BTreeMap::new();
    let result = guardrail(&base)
        .apply(
            inputs(&["first", "second", "third"]),
            &mut request_data,
            None,
        )
        .await;
    assert!(matches!(
        result,
        Err(GuardrailFailure::GuardrailRaised {
            status_code: 400,
            ..
        })
    ));
    let captured = state.captured.lock().unwrap();
    assert_eq!(captured.len(), 2);
    drop(captured);
    let records = record_objects(&request_data);
    assert_eq!(records.len(), 2);
    assert_eq!(field(&records[0], "guardrail_status"), text("success"));
    assert_eq!(
        field(&records[1], "guardrail_status"),
        text("guardrail_intervened")
    );
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_http_failure_is_not_an_intervention() {
    let (base, _state, mut server) = serve(vec![Reply {
        status: StatusCode::FORBIDDEN,
        body: r#"{"detail":"forbidden"}"#.to_owned(),
    }])
    .await;
    let mut request_data = BTreeMap::new();
    let error = guardrail(&base)
        .apply(inputs(&["hello"]), &mut request_data, None)
        .await
        .expect_err("http failure");
    assert!(
        matches!(&error, GuardrailFailure::Other(message) if message.contains("HTTP status 403"))
    );
    assert!(!error.is_intervention());
    let records = record_objects(&request_data);
    assert_eq!(records.len(), 1);
    assert_eq!(
        field(&records[0], "guardrail_status"),
        text("guardrail_failed_to_respond")
    );
    assert_eq!(field(&records[0], "guardrail_provider"), text("enkryptai"));
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_records_non_json_response_as_failed() {
    let (base, _state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body: "not json".to_owned(),
    }])
    .await;
    let mut request_data = BTreeMap::new();
    let error = guardrail(&base)
        .apply(inputs(&["hello"]), &mut request_data, None)
        .await
        .expect_err("response is not json");
    assert!(matches!(&error, GuardrailFailure::Other(message) if message.contains("not JSON")));
    let records = record_objects(&request_data);
    assert_eq!(
        field(&records[0], "guardrail_status"),
        text("guardrail_failed_to_respond")
    );
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_records_malformed_response_as_failed() {
    let (base, _state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body: "[1,2]".to_owned(),
    }])
    .await;
    let mut request_data = BTreeMap::new();
    let error = guardrail(&base)
        .apply(inputs(&["hello"]), &mut request_data, None)
        .await
        .expect_err("response is not an object");
    assert!(
        matches!(&error, GuardrailFailure::Other(message) if message.contains("not a JSON object"))
    );
    let records = record_objects(&request_data);
    assert_eq!(
        field(&records[0], "guardrail_response"),
        parse_value("[1,2]")
    );
    assert_eq!(
        field(&records[0], "guardrail_status"),
        text("guardrail_failed_to_respond")
    );
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_rejects_a_mistyped_summary() {
    let (base, _state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body: r#"{"summary":42}"#.to_owned(),
    }])
    .await;
    let mut request_data = BTreeMap::new();
    let error = guardrail(&base)
        .apply(inputs(&["hello"]), &mut request_data, None)
        .await
        .expect_err("summary is not an object");
    assert!(
        matches!(&error, GuardrailFailure::Other(message) if message.contains("detection shape"))
    );
    let records = record_objects(&request_data);
    assert_eq!(
        field(&records[0], "guardrail_status"),
        text("guardrail_failed_to_respond")
    );
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_bounds_the_response_size() {
    let body = format!("{{}}{}", " ".repeat(1024 * 1024));
    let (base, _state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body,
    }])
    .await;
    let error = guardrail(&base)
        .apply(inputs(&["hello"]), &mut BTreeMap::new(), None)
        .await
        .expect_err("oversized response");
    assert!(
        matches!(&error, GuardrailFailure::Other(message) if message.contains("exceeds 1048576 bytes"))
    );
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_accepts_a_response_at_the_size_limit() {
    let body = format!(r#"{{"summary":{{}}}}{}"#, " ".repeat(1024 * 1024 - 14));
    let (base, _state, mut server) = serve(vec![Reply {
        status: StatusCode::OK,
        body,
    }])
    .await;
    let mut request_data = BTreeMap::new();
    guardrail(&base)
        .apply(inputs(&["hello"]), &mut request_data, None)
        .await
        .expect("at-limit response");
    server.shutdown().await;
}

#[tokio::test]
async fn enkryptai_guardrail_rejects_an_invalid_policy_header() {
    let (base, state, mut server) = serve(vec![]).await;
    let mut settings = settings_for(&base);
    settings.policy_name = Some("bad\nvalue".to_owned());
    let mut request_data = BTreeMap::new();
    let error = guardrail_with(settings)
        .apply(inputs(&["hello"]), &mut request_data, None)
        .await
        .expect_err("invalid header");
    assert!(
        matches!(&error, GuardrailFailure::Other(message) if message.contains("x-enkrypt-policy"))
    );
    assert!(state.captured.lock().unwrap().is_empty());
    let records = record_objects(&request_data);
    assert_eq!(
        field(&records[0], "guardrail_status"),
        text("guardrail_failed_to_respond")
    );
    server.shutdown().await;
}

#[rstest]
#[case::toxicity_list(r#"{"summary":{"toxicity":["toxic"]}}"#, true)]
#[case::empty_toxicity_list(r#"{"summary":{"toxicity":[]}}"#, false)]
#[case::policy_violation(r#"{"summary":{"policy_violation":1}}"#, true)]
#[case::zero_policy_violation(r#"{"summary":{"policy_violation":0}}"#, false)]
#[case::pii(r#"{"summary":{"pii":1}}"#, true)]
#[case::keyword(r#"{"summary":{"keyword_detected":1}}"#, true)]
#[case::bias(r#"{"summary":{"bias":1}}"#, true)]
#[case::prompt_injection(r#"{"summary":{"prompt_injection":1}}"#, true)]
#[case::jailbreak(r#"{"summary":{"jailbreak":1}}"#, true)]
#[case::unknown_integer(r#"{"summary":{"nsfw":1},"details":{"nsfw":{"score":0.9}}}"#, true)]
#[case::unknown_zero(r#"{"summary":{"nsfw":0}}"#, false)]
#[case::unknown_float_one(r#"{"summary":{"nsfw":1.0}}"#, true)]
#[case::missing_fields(r#"{}"#, false)]
fn enkryptai_guardrail_processes_the_summary(#[case] body: &str, #[case] detected: bool) {
    let value = parse_value(body);
    let response = parsed_response(&value);
    let processed: Processed = process(&response);
    assert_eq!(!processed.attacks.is_empty(), detected);
}

#[test]
fn enkryptai_guardrail_collects_the_detected_attacks_in_order() {
    let body = r#"{"summary":{"policy_violation":1,"pii":1,"toxicity":["toxic"],"keyword_detected":1,"bias":1,"prompt_injection":1,"jailbreak":1},"details":{"policy_violation":{"violating_policy":"p","explanation":"e"},"pii":{"pii":{"email":1}},"toxicity":{"toxic":0.9},"keyword_detected":{"detected_keywords":["k"]},"bias":{"bias_detected":true},"prompt_injection":{"score":0.8},"jailbreak":{"score":0.9}}}"#;
    let value = parse_value(body);
    let response = parsed_response(&value);
    let processed = process(&response);
    assert_eq!(
        processed.attacks,
        vec![
            "toxicity",
            "policy_violation",
            "pii",
            "keyword_detected",
            "bias",
            "prompt_injection",
            "jailbreak"
        ]
    );
}

#[test]
fn enkryptai_guardrail_processes_unknown_integer_detectors() {
    let zero = parsed_response(&parse_value(r#"{"summary":{"nsfw":0}}"#));
    assert!(process(&zero).attacks.is_empty());
    let one = parsed_response(&parse_value(
        r#"{"summary":{"nsfw":1},"details":{"nsfw":{"score":0.9}}}"#,
    ));
    let processed = process(&one);
    assert_eq!(processed.attacks, vec!["nsfw"]);
    let float = parsed_response(&parse_value(r#"{"summary":{"nsfw":1.0}}"#));
    assert_eq!(process(&float).attacks, vec!["nsfw"]);
}

#[test]
fn enkryptai_config_loads_defaults_and_the_environment_key() {
    let lookup = BTreeMap::from([("ENKRYPTAI_API_KEY".to_owned(), "env-key".to_owned())]);
    let source =
        "guardrails:\n  - guardrail_name: guard\n    litellm_params:\n      guardrail: enkryptai\n";
    let config = ConfigLoader { lookup }.load(source).expect("config");
    let configured = &config.sections.enkryptai_detection.guardrails[0];
    assert_eq!(configured.api_key.expose_secret(), "env-key");
    assert_eq!(configured.api_base, DEFAULT_API_BASE);
    assert_eq!(configured.policy_name, None);
    assert!(configured.block_on_violation);
    assert_eq!(configured.guardrail_name, "guard");
}

#[test]
fn enkryptai_config_reads_the_api_base_environment() {
    let lookup = BTreeMap::from([
        ("ENKRYPTAI_API_KEY".to_owned(), "env-key".to_owned()),
        (
            "ENKRYPTAI_API_BASE".to_owned(),
            "http://enkrypt.test".to_owned(),
        ),
    ]);
    let source =
        "guardrails:\n  - guardrail_name: guard\n    litellm_params:\n      guardrail: enkryptai\n";
    let config = ConfigLoader { lookup }.load(source).expect("config");
    let configured = &config.sections.enkryptai_detection.guardrails[0];
    assert_eq!(configured.api_base, "http://enkrypt.test");
}

#[test]
fn enkryptai_config_reads_the_timeout() {
    let source = "guardrails:\n  - guardrail_name: guard\n    litellm_params:\n      guardrail: enkryptai\n      api_key: key\n      timeout: '2.5'\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config");
    assert_eq!(
        config.sections.enkryptai_detection.guardrails[0].timeout,
        Duration::from_millis(2500)
    );
    let defaults = "guardrails:\n  - guardrail_name: guard\n    litellm_params:\n      guardrail: enkryptai\n      api_key: key\n";
    let loaded = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(defaults)
    .expect("config");
    assert_eq!(
        loaded.sections.enkryptai_detection.guardrails[0].timeout,
        Duration::from_secs(600)
    );
}

#[test]
fn enkryptai_config_rejects_a_missing_api_key() {
    let source =
        "guardrails:\n  - guardrail_name: guard\n    litellm_params:\n      guardrail: enkryptai\n";
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect_err("missing key");
    assert!(error.to_string().contains("ENKRYPTAI_API_KEY"));
}

#[test]
fn enkryptai_config_uses_the_environment_for_an_empty_configured_key() {
    let lookup = BTreeMap::from([("ENKRYPTAI_API_KEY".to_owned(), "env-key".to_owned())]);
    let source = "guardrails:\n  - guardrail_name: guard\n    litellm_params:\n      guardrail: enkryptai\n      api_key: ''\n";
    let config = ConfigLoader { lookup }.load(source).expect("config");
    assert_eq!(
        config.sections.enkryptai_detection.guardrails[0]
            .api_key
            .expose_secret(),
        "env-key"
    );
}

#[test]
fn enkryptai_config_rejects_an_invalid_timeout() {
    let source = "guardrails:\n  - guardrail_name: guard\n    litellm_params:\n      guardrail: enkryptai\n      api_key: key\n      timeout: nope\n";
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect_err("invalid timeout");
    assert!(error.to_string().contains("timeout must be numeric"));
}

#[test]
fn enkryptai_config_treats_an_empty_environment_value_as_unset() {
    let lookup = BTreeMap::from([("ENKRYPTAI_API_KEY".to_owned(), String::new())]);
    let source =
        "guardrails:\n  - guardrail_name: guard\n    litellm_params:\n      guardrail: enkryptai\n";
    let error = ConfigLoader { lookup }.load(source).expect_err("empty key");
    assert!(error.to_string().contains("ENKRYPTAI_API_KEY"));
}

#[test]
fn enkryptai_config_accepts_the_unused_detection_fields() {
    let source = "guardrails:\n  - guardrail_name: guard\n    litellm_params:\n      guardrail: enkryptai\n      api_key: key\n      policy_name: policy-a\n      deployment_name: deploy\n      block_on_violation: false\n      detectors:\n        nsfw:\n          enabled: true\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config");
    let configured = &config.sections.enkryptai_detection.guardrails[0];
    assert_eq!(configured.policy_name.as_deref(), Some("policy-a"));
    assert_eq!(configured.deployment_name.as_deref(), Some("deploy"));
    assert!(!configured.block_on_violation);
    assert!(configured.detectors.is_some());
}

#[test]
fn enkryptai_config_treats_an_empty_policy_name_as_unset() {
    let source = "guardrails:\n  - guardrail_name: guard\n    litellm_params:\n      guardrail: enkryptai\n      api_key: key\n      policy_name: ''\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config");
    assert_eq!(
        config.sections.enkryptai_detection.guardrails[0].policy_name,
        None
    );
}

#[test]
fn enkryptai_config_never_shows_the_api_key_in_debug() {
    let lookup = BTreeMap::from([("ENKRYPTAI_API_KEY".to_owned(), "super-secret".to_owned())]);
    let source =
        "guardrails:\n  - guardrail_name: guard\n    litellm_params:\n      guardrail: enkryptai\n";
    let config = ConfigLoader { lookup }.load(source).expect("config");
    let configured = &config.sections.enkryptai_detection.guardrails[0];
    assert!(!format!("{configured:?}").contains("super-secret"));
}

#[test]
fn enkryptai_config_keeps_other_guardrails_out() {
    let source = "guardrails:\n  - guardrail_name: other\n    litellm_params:\n      guardrail: lasso\n      api_key: key\n  - guardrail_name: guard\n    litellm_params:\n      guardrail: enkryptai\n      api_key: key\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config");
    let guardrails = &config.sections.enkryptai_detection.guardrails;
    assert_eq!(guardrails.len(), 1);
    assert_eq!(guardrails[0].guardrail_name, "guard");
}
