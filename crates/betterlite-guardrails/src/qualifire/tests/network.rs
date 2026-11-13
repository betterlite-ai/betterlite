use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::{NoSecrets, args, inputs, json, text};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::qualifire::QualifireGuardrail;
use crate::qualifire::error::QualifireError;
use crate::qualifire::init::QualifireArgs;
use crate::qualifire::wire::QualifireResponse;

// Hardcoded independently of `MAX_RESPONSE_BYTES`, so a mutation of that constant can't move this boundary too.
const SPEC_MAX_RESPONSE_BYTES: usize = 0x0010_0000;
const CLEAN: &str = r#"{"score": 90, "status": "passed", "evaluationResults": [{"results": [{"flagged": false}]}]}"#;
const FLAGGED: &str =
    r#"{"score": 12, "status": "failed", "evaluationResults": [{"results": [{"flagged": true}]}]}"#;

struct Seen {
    path: String,
    api_key: Option<String>,
    content_type: Option<String>,
    body: Vec<u8>,
}

struct Mock {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinSet<io::Result<()>>,
}

impl Mock {
    fn only_request(&self) -> Seen {
        let mut seen = self.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "the guardrail sends exactly one request");
        seen.remove(0)
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .map(|value| value.to_str().unwrap().to_owned())
}

async fn mock(status: StatusCode, body: String, delay: Duration) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let answer = move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let log = Arc::clone(&log);
        let body = body.clone();
        async move {
            log.lock().unwrap().push(Seen {
                path: uri.path().to_owned(),
                api_key: header(&headers, "x-qualifire-api-key"),
                content_type: header(&headers, "content-type"),
                body: bytes.to_vec(),
            });
            tokio::time::sleep(delay).await;
            (status, body)
        }
    };
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, Router::new().fallback(answer)).into_future());
    Mock {
        base,
        seen,
        _server: server,
    }
}

async fn answering(body: &str) -> Mock {
    mock(StatusCode::OK, body.to_owned(), Duration::ZERO).await
}

fn guardrail(mock: &Mock, mut args: QualifireArgs) -> QualifireGuardrail {
    args.api_base = Some(mock.base.clone());
    QualifireGuardrail::new(
        &args,
        &NoSecrets,
        &BTreeMap::new(),
        reqwest::Client::default(),
    )
    .unwrap()
}

fn user_text(content: &str) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(vec![content.to_owned()]),
        ..inputs()
    }
}

fn sent(seen: &Seen) -> GuardrailValue {
    json(std::str::from_utf8(&seen.body).unwrap())
}

fn dynamic(fields: &str) -> BTreeMap<String, GuardrailValue> {
    let body = format!(r#"{{"guardrails": [{{"qualifire": {{"extra_body": {fields}}}}}]}}"#);
    match json(&body) {
        GuardrailValue::Object(object) => object,
        _ => panic!("the test request data is not a JSON object"),
    }
}

async fn run(
    guardrail: &QualifireGuardrail,
    scan: GuardrailApiInputs,
    request_data: &BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
) -> Result<GuardrailApiInputs, QualifireError> {
    guardrail
        .apply_guardrail(scan, request_data, input_type)
        .await
}

async fn run_request(
    guardrail: &QualifireGuardrail,
    scan: GuardrailApiInputs,
) -> Result<GuardrailApiInputs, QualifireError> {
    run(
        guardrail,
        scan,
        &BTreeMap::new(),
        GuardrailInputType::Request,
    )
    .await
}

#[tokio::test]
async fn a_clean_evaluation_posts_to_evaluate_and_returns_the_inputs() {
    let server = answering(CLEAN).await;
    let mut args = args();
    args.api_key = Some(SecretString::from("key-1".to_owned()));
    args.pii_check = Some(true);
    let scan = user_text("hello");
    let returned = run_request(&guardrail(&server, args), scan.clone())
        .await
        .unwrap();
    assert_eq!(returned, scan);
    let seen = server.only_request();
    assert_eq!(seen.path, "/api/evaluation/evaluate");
    assert_eq!(seen.api_key.as_deref(), Some("key-1"));
    assert_eq!(seen.content_type.as_deref(), Some("application/json"));
    assert_eq!(
        sent(&seen),
        json(r#"{"messages": [{"role": "user", "content": "hello"}], "pii_check": true}"#)
    );
}

#[tokio::test]
async fn an_evaluation_id_posts_to_invoke() {
    let server = answering(CLEAN).await;
    let mut args = args();
    args.evaluation_id = Some("eval-1".to_owned());
    run_request(&guardrail(&server, args), user_text("hello"))
        .await
        .unwrap();
    let seen = server.only_request();
    assert_eq!(seen.path, "/api/evaluation/invoke");
    assert_eq!(
        sent(&seen),
        json(
            r#"{"evaluation_id": "eval-1", "input": "hello", "output": "", "messages": [{"role": "user", "content": "hello"}]}"#
        )
    );
}

#[tokio::test]
async fn a_response_check_sends_the_last_text_as_the_output() {
    let server = answering(CLEAN).await;
    let mut args = args();
    args.pii_check = Some(true);
    run(
        &guardrail(&server, args),
        user_text("the answer"),
        &BTreeMap::new(),
        GuardrailInputType::Response,
    )
    .await
    .unwrap();
    assert_eq!(
        sent(&server.only_request()),
        json(
            r#"{"messages": [{"role": "assistant", "content": "the answer"}], "output": "the answer", "pii_check": true}"#
        )
    );
}

#[tokio::test]
async fn request_tools_reach_the_tool_selection_check() {
    let server = answering(CLEAN).await;
    let mut args = args();
    args.tool_selection_quality_check = Some(true);
    let request_data = BTreeMap::from([(
        "tools".to_owned(),
        json(r#"[{"type": "function", "function": {"name": "lookup"}}]"#),
    )]);
    run(
        &guardrail(&server, args),
        user_text("hello"),
        &request_data,
        GuardrailInputType::Request,
    )
    .await
    .unwrap();
    assert_eq!(
        sent(&server.only_request()),
        json(
            r#"{"messages": [{"role": "user", "content": "hello"}], "tool_selection_quality_check": true, "available_tools": [{"name": "lookup", "description": "", "parameters": {}}]}"#
        )
    );
}

#[tokio::test]
async fn a_flagged_answer_blocks_with_the_qualifire_response() {
    let server = answering(FLAGGED).await;
    let result = run_request(&guardrail(&server, args()), user_text("hello")).await;
    let Err(QualifireError::Blocked { detail }) = result else {
        panic!("a flagged answer blocks the request");
    };
    assert_eq!(detail.error, "Violated guardrail policy");
    assert_eq!(
        detail.qualifire_response,
        QualifireResponse {
            score: json("12"),
            status: text("failed")
        }
    );
}

#[tokio::test]
async fn a_flagged_answer_passes_in_monitor_mode() {
    let server = answering(FLAGGED).await;
    let mut args = args();
    args.on_flagged = Some("monitor".to_owned());
    let scan = user_text("hello");
    let returned = run_request(&guardrail(&server, args), scan.clone())
        .await
        .unwrap();
    assert_eq!(returned, scan);
}

#[tokio::test]
async fn dynamic_params_override_the_evaluation_and_the_flag_handling() {
    let server = answering(FLAGGED).await;
    let mut args = args();
    args.evaluation_id = Some("static-id".to_owned());
    let request_data = dynamic(r#"{"evaluation_id": "dynamic-id", "on_flagged": "monitor"}"#);
    run(
        &guardrail(&server, args),
        user_text("hello"),
        &request_data,
        GuardrailInputType::Request,
    )
    .await
    .unwrap();
    let seen = server.only_request();
    assert_eq!(seen.path, "/api/evaluation/invoke");
    assert_eq!(
        sent(&seen),
        json(
            r#"{"evaluation_id": "dynamic-id", "input": "hello", "output": "", "messages": [{"role": "user", "content": "hello"}]}"#
        )
    );
}

#[tokio::test]
async fn a_dynamic_block_overrides_monitor_settings() {
    let server = answering(FLAGGED).await;
    let mut args = args();
    args.on_flagged = Some("monitor".to_owned());
    let request_data = dynamic(r#"{"on_flagged": "block"}"#);
    let result = run(
        &guardrail(&server, args),
        user_text("hello"),
        &request_data,
        GuardrailInputType::Request,
    )
    .await;
    assert!(matches!(result, Err(QualifireError::Blocked { .. })));
}

#[tokio::test]
async fn dynamic_assertions_replace_the_configured_ones() {
    let server = answering(CLEAN).await;
    let mut args = args();
    args.assertions = Some(vec!["static rule".to_owned()]);
    let request_data = dynamic(r#"{"assertions": ["dynamic rule"]}"#);
    run(
        &guardrail(&server, args),
        user_text("hello"),
        &request_data,
        GuardrailInputType::Request,
    )
    .await
    .unwrap();
    assert_eq!(
        sent(&server.only_request()),
        json(
            r#"{"messages": [{"role": "user", "content": "hello"}], "assertions": ["dynamic rule"]}"#
        )
    );
}

#[tokio::test]
async fn configured_assertions_are_sent_without_a_dynamic_override() {
    let server = answering(CLEAN).await;
    let mut args = args();
    args.assertions = Some(vec!["static rule".to_owned()]);
    run_request(&guardrail(&server, args), user_text("hello"))
        .await
        .unwrap();
    assert_eq!(
        sent(&server.only_request()),
        json(
            r#"{"messages": [{"role": "user", "content": "hello"}], "assertions": ["static rule"]}"#
        )
    );
}

#[tokio::test]
async fn a_missing_api_key_sends_an_empty_header() {
    let server = answering(CLEAN).await;
    run_request(&guardrail(&server, args()), user_text("hello"))
        .await
        .unwrap();
    assert_eq!(server.only_request().api_key.as_deref(), Some(""));
}

#[tokio::test]
async fn an_api_key_that_is_not_a_header_value_is_refused_before_sending() {
    let server = answering(CLEAN).await;
    let mut args = args();
    args.api_key = Some(SecretString::from("bad\nkey".to_owned()));
    let result = run_request(&guardrail(&server, args), user_text("hello")).await;
    assert!(matches!(result, Err(QualifireError::InvalidHeader { .. })));
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_failing_status_is_returned_not_failed_open() {
    let server = mock(
        StatusCode::INTERNAL_SERVER_ERROR,
        "oops".to_owned(),
        Duration::ZERO,
    )
    .await;
    let result = run_request(&guardrail(&server, args()), user_text("hello")).await;
    assert!(matches!(
        result,
        Err(QualifireError::HttpStatus { status }) if status == StatusCode::INTERNAL_SERVER_ERROR
    ));
}

#[tokio::test]
async fn a_slow_answer_times_out() {
    let server = mock(StatusCode::OK, CLEAN.to_owned(), Duration::from_secs(30)).await;
    let mut args = args();
    args.timeout = Some(0.2);
    let result = run_request(&guardrail(&server, args), user_text("hello")).await;
    assert!(matches!(result, Err(QualifireError::Timeout { .. })));
}

#[tokio::test]
async fn an_unreachable_server_is_a_request_error() {
    let address = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let mut args = args();
    args.api_base = Some(format!("http://{address}"));
    let unreachable = QualifireGuardrail::new(
        &args,
        &NoSecrets,
        &BTreeMap::new(),
        reqwest::Client::default(),
    )
    .unwrap();
    let result = run_request(&unreachable, user_text("hello")).await;
    assert!(matches!(result, Err(QualifireError::Request { .. })));
}

#[tokio::test]
async fn a_body_that_is_not_json_is_a_response_error() {
    let server = answering("nope").await;
    let result = run_request(&guardrail(&server, args()), user_text("hello")).await;
    assert!(matches!(result, Err(QualifireError::ResponseJson { .. })));
}

#[tokio::test]
async fn a_body_at_the_size_limit_is_read() {
    let body = format!("{{}}{}", " ".repeat(SPEC_MAX_RESPONSE_BYTES - 2));
    assert_eq!(body.len(), SPEC_MAX_RESPONSE_BYTES);
    let server = answering(&body).await;
    run_request(&guardrail(&server, args()), user_text("hello"))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_body_over_the_size_limit_is_refused() {
    let body = format!("{{}}{}", " ".repeat(SPEC_MAX_RESPONSE_BYTES - 1));
    assert_eq!(body.len(), SPEC_MAX_RESPONSE_BYTES + 1);
    let server = answering(&body).await;
    let result = run_request(&guardrail(&server, args()), user_text("hello")).await;
    assert!(matches!(
        result,
        Err(QualifireError::ResponseTooLarge { limit }) if limit == SPEC_MAX_RESPONSE_BYTES
    ));
}
