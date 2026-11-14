#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::invocation::call_or_fail_open;
use crate::crowdstrike_aidr::error::CrowdStrikeError;
use crate::crowdstrike_aidr::handler::{ConstructorArgs, CrowdStrikeAidr, Dependencies};
use crate::crowdstrike_aidr::models::{
    EventType, GuardInput, GuardPayload, Message, MessageContent,
};
use crate::crowdstrike_streaming::StreamingParams;
use crate::generic_guardrail_contracts::{GuardrailInputType, GuardrailToolParam};
use crate::guardrail_base::contract::GuardrailContract as _;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::record::{GuardrailInvocationRecord, GuardrailSpanSink};

struct DiscardSink;

impl GuardrailSpanSink for DiscardSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

fn handler(api_base: &str, fail_on_error: Option<bool>) -> CrowdStrikeAidr {
    let args = ConstructorArgs {
        guardrail_name: Some("cs-aidr".to_owned()),
        api_key: Some("test-key".to_owned()),
        api_base: Some(api_base.to_owned()),
        fail_on_error,
        streaming: StreamingParams::default(),
        timeout: None,
    };
    CrowdStrikeAidr::new(
        &args,
        &BTreeMap::<String, String>::new(),
        Dependencies {
            client: reqwest::Client::default(),
            sink: Box::new(DiscardSink),
            now: SystemTime::now,
        },
    )
    .expect("the constructor gets both secrets")
}

fn payload() -> GuardPayload {
    GuardPayload {
        guard_input: GuardInput {
            messages: vec![Message {
                role: "user".to_owned(),
                content: Some(MessageContent::Text("hi".to_owned())),
            }],
            tools: Some(Vec::new()),
        },
        event_type: EventType::Input,
        model: None,
        user_id: None,
        extra_info: None,
    }
}

fn inputs() -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(vec!["hi".to_owned()]),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

async fn serve(
    status: StatusCode,
    body: &'static str,
) -> (String, Arc<Mutex<Vec<u8>>>, JoinSet<std::io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let captured = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    let answer = move |bytes: Bytes| {
        let sink = Arc::clone(&sink);
        async move {
            *sink.lock().unwrap() = bytes.to_vec();
            (status, body)
        }
    };
    let router = Router::new().route("/v1/guard_chat_completions", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, captured, server)
}

fn captured(captured: &Arc<Mutex<Vec<u8>>>) -> CapturedPayload {
    serde_json::from_slice(&captured.lock().unwrap()).expect("the payload is the guard body")
}

#[derive(serde::Deserialize)]
struct CapturedPayload {
    guard_input: GuardInput,
    event_type: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    extra_info: Option<BTreeMap<String, String>>,
}

#[tokio::test]
async fn _call_or_fail_open_success() {
    let (base, _captured, _server) =
        serve(StatusCode::OK, r#"{"result": {"transformed": false}}"#).await;
    let guardrail = handler(&base, Some(true));
    let mut request_data = BTreeMap::new();
    let result = call_or_fail_open(&guardrail, &payload(), "test", &mut request_data, None)
        .await
        .expect("the call succeeds");
    assert_eq!(result.transformed, Some(false));
}

#[tokio::test]
async fn _call_or_fail_open_http_exception() {
    let (base, _captured, _server) =
        serve(StatusCode::OK, r#"{"result": {"blocked": true}}"#).await;
    let guardrail = handler(&base, Some(false));
    let mut request_data = BTreeMap::new();
    let error = call_or_fail_open(&guardrail, &payload(), "test", &mut request_data, None)
        .await
        .expect_err("a blocked verdict raises even with fail_on_error false");
    assert!(matches!(error, CrowdStrikeError::Blocked { .. }));
}

#[tokio::test]
async fn _call_or_fail_open_fail_closed() {
    let (base, _captured, _server) = serve(StatusCode::INTERNAL_SERVER_ERROR, "oops").await;
    let guardrail = handler(&base, Some(true));
    let mut request_data = BTreeMap::new();
    let error = call_or_fail_open(&guardrail, &payload(), "test", &mut request_data, None)
        .await
        .expect_err("fail_on_error true passes the failure up");
    assert!(matches!(error, CrowdStrikeError::HttpStatus { .. }));
}

#[tokio::test]
async fn _call_or_fail_open_fail_open() {
    let (base, _captured, _server) = serve(StatusCode::INTERNAL_SERVER_ERROR, "oops").await;
    let guardrail = handler(&base, Some(false));
    let mut request_data = BTreeMap::new();
    let result = call_or_fail_open(&guardrail, &payload(), "test", &mut request_data, None)
        .await
        .expect("fail_on_error false fails open");
    assert_eq!(
        result,
        crate::crowdstrike_aidr::models::GuardResult::default()
    );
    let Some(GuardrailValue::Object(bucket)) = request_data.get("metadata") else {
        panic!("the fail-open call records guardrail information");
    };
    let Some(GuardrailValue::Array(records)) = bucket.get("standard_logging_guardrail_information")
    else {
        panic!("the record sits in the standard logging list");
    };
    let Some(GuardrailValue::Object(entry)) = records.first() else {
        panic!("the record is an object");
    };
    assert_eq!(
        entry.get("guardrail_status"),
        Some(&GuardrailValue::Text(
            "guardrail_failed_to_respond".to_owned()
        ))
    );
    let Some(GuardrailValue::Object(response)) = entry.get("guardrail_response") else {
        panic!("the record keeps the error as an object");
    };
    assert!(matches!(
        response.get("error"),
        Some(GuardrailValue::Text(text)) if text.contains("status 500")
    ));
}

static TICKS: AtomicU64 = AtomicU64::new(0);

fn ticking_clock() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(100 + 5 * TICKS.fetch_add(1, Ordering::SeqCst))
}

#[tokio::test]
async fn _call_or_fail_open_records_the_failure_timing() {
    let (base, _captured, _server) = serve(StatusCode::INTERNAL_SERVER_ERROR, "oops").await;
    let mut guardrail = handler(&base, Some(false));
    guardrail.now = ticking_clock;
    let mut request_data = BTreeMap::new();
    call_or_fail_open(&guardrail, &payload(), "test", &mut request_data, None)
        .await
        .expect("fail_on_error false fails open");
    let Some(GuardrailValue::Object(bucket)) = request_data.get("metadata") else {
        panic!("the fail-open call records guardrail information");
    };
    let Some(GuardrailValue::Array(records)) = bucket.get("standard_logging_guardrail_information")
    else {
        panic!("the record sits in the standard logging list");
    };
    let Some(GuardrailValue::Object(entry)) = records.first() else {
        panic!("the record is an object");
    };
    assert_eq!(
        entry.get("start_time"),
        Some(&GuardrailValue::Decimal(100.0))
    );
    assert_eq!(entry.get("end_time"), Some(&GuardrailValue::Decimal(105.0)));
    assert_eq!(entry.get("duration"), Some(&GuardrailValue::Decimal(5.0)));
}

#[tokio::test]
async fn apply_guardrail_request_input() {
    let (base, _captured, _server) = serve(
        StatusCode::OK,
        r#"{"result": {"transformed": true, "guard_output": {"messages": [{"role": "user", "content": "masked"}]}}}"#,
    )
    .await;
    let guardrail = handler(&base, None);
    let request = GuardrailApiInputs {
        structured_messages: Some(vec![crate::generic_guardrail_contracts::GuardrailMessage {
            role: "user".to_owned(),
            extra: BTreeMap::from([(
                "content".to_owned(),
                GuardrailValue::Text("hello".to_owned()),
            )]),
        }]),
        ..inputs()
    };
    let mut request_data = BTreeMap::new();
    let output = guardrail
        .apply_guardrail(
            request,
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("the request path succeeds");
    assert_eq!(output.texts, Some(vec!["masked".to_owned()]));
    let messages = output
        .structured_messages
        .expect("structured messages come back");
    assert_eq!(
        messages[0].extra.get("content"),
        Some(&GuardrailValue::Text("masked".to_owned()))
    );
}

#[tokio::test]
async fn apply_guardrail_response_input() {
    let (base, _captured, _server) = serve(
        StatusCode::OK,
        r#"{"result": {"transformed": true, "guard_output": {"messages": [{"role": "assistant", "content": "masked answer"}]}}}"#,
    )
    .await;
    let guardrail = handler(&base, None);
    let request = GuardrailApiInputs {
        texts: Some(vec!["original answer".to_owned()]),
        ..inputs()
    };
    let mut request_data = BTreeMap::new();
    let output = guardrail
        .apply_guardrail(
            request,
            &mut request_data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect("the response path succeeds");
    assert_eq!(output.texts, Some(vec!["masked answer".to_owned()]));
    assert_eq!(output.structured_messages, None);
}

#[tokio::test]
async fn apply_guardrail_includes_model_in_payload() {
    let (base, captured, _server) =
        serve(StatusCode::OK, r#"{"result": {"transformed": false}}"#).await;
    let guardrail = handler(&base, None);
    let request = GuardrailApiInputs {
        model: Some("gpt-4o".to_owned()),
        ..inputs()
    };
    let mut request_data = BTreeMap::new();
    guardrail
        .apply_guardrail(
            request,
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("the call succeeds");
    let body = captured(&captured);
    assert_eq!(body.model, Some("gpt-4o".to_owned()));
    assert_eq!(body.event_type, "input");
    assert_eq!(body.guard_input.messages.len(), 1);
}

#[tokio::test]
async fn apply_guardrail_includes_user_id_in_payload() {
    let (base, captured, _server) =
        serve(StatusCode::OK, r#"{"result": {"transformed": false}}"#).await;
    let guardrail = handler(&base, None);
    let mut request_data = BTreeMap::from([(
        "metadata".to_owned(),
        GuardrailValue::Object(BTreeMap::from([(
            "user_api_key_user_id".to_owned(),
            GuardrailValue::Text("u-1".to_owned()),
        )])),
    )]);
    guardrail
        .apply_guardrail(
            inputs(),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("the call succeeds");
    let body = captured(&captured);
    assert_eq!(body.user_id, Some("u-1".to_owned()));
}

#[tokio::test]
async fn apply_guardrail_includes_user_email_in_extra_info() {
    let (base, captured, _server) =
        serve(StatusCode::OK, r#"{"result": {"transformed": false}}"#).await;
    let guardrail = handler(&base, None);
    let mut request_data = BTreeMap::from([(
        "metadata".to_owned(),
        GuardrailValue::Object(BTreeMap::from([(
            "user_api_key_user_email".to_owned(),
            GuardrailValue::Text("user@example.com".to_owned()),
        )])),
    )]);
    guardrail
        .apply_guardrail(
            inputs(),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("the call succeeds");
    let body = captured(&captured);
    assert_eq!(
        body.extra_info,
        Some(BTreeMap::from([(
            "user_name".to_owned(),
            "user@example.com".to_owned()
        )]))
    );
}

#[tokio::test]
async fn apply_guardrail_returns_inputs_unchanged() {
    let (base, _captured, _server) =
        serve(StatusCode::OK, r#"{"result": {"transformed": false}}"#).await;
    let guardrail = handler(&base, None);
    let request = GuardrailApiInputs {
        model: Some("gpt-4o".to_owned()),
        ..inputs()
    };
    let mut request_data = BTreeMap::new();
    let output = guardrail
        .apply_guardrail(
            request.clone(),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("the call succeeds");
    assert_eq!(output, request);
}

#[tokio::test]
async fn apply_guardrail_adds_to_applied_guardrails_header() {
    let (base, _captured, _server) =
        serve(StatusCode::OK, r#"{"result": {"transformed": false}}"#).await;
    let guardrail = handler(&base, None);
    let mut request_data =
        BTreeMap::from([("messages".to_owned(), GuardrailValue::Array(Vec::new()))]);
    guardrail
        .apply_guardrail(
            inputs(),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("the call succeeds");
    let Some(GuardrailValue::Object(bucket)) = request_data.get("metadata") else {
        panic!("the metadata bucket exists");
    };
    assert_eq!(
        bucket.get("applied_guardrails"),
        Some(&GuardrailValue::Array(vec![GuardrailValue::Text(
            "cs-aidr".to_owned()
        )]))
    );
}

#[tokio::test]
async fn apply_guardrail_without_a_request_body_skips_the_applied_list() {
    let (base, _captured, _server) =
        serve(StatusCode::OK, r#"{"result": {"transformed": false}}"#).await;
    let guardrail = handler(&base, None);
    let mut request_data = BTreeMap::new();
    guardrail
        .apply_guardrail(
            inputs(),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("the call succeeds");
    assert!(!request_data.contains_key("metadata"));
}

fn applied_bucket(names: &[&str], keep: &str) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        (
            "applied_guardrails".to_owned(),
            GuardrailValue::Array(
                names
                    .iter()
                    .map(|name| GuardrailValue::Text((*name).to_owned()))
                    .collect(),
            ),
        ),
        ("keep".to_owned(), GuardrailValue::Text(keep.to_owned())),
    ]))
}

#[tokio::test]
async fn apply_guardrail_lists_the_applied_name_once_in_litellm_metadata() {
    let (base, _captured, _server) =
        serve(StatusCode::OK, r#"{"result": {"transformed": false}}"#).await;
    let guardrail = handler(&base, None);
    let mut seen = BTreeMap::from([
        ("messages".to_owned(), GuardrailValue::Array(Vec::new())),
        (
            "litellm_metadata".to_owned(),
            applied_bucket(&["cs-aidr"], "a"),
        ),
        ("metadata".to_owned(), applied_bucket(&[], "b")),
    ]);
    guardrail
        .apply_guardrail(inputs(), &mut seen, GuardrailInputType::Request, None)
        .await
        .expect("the call succeeds");
    assert_eq!(
        seen.get("litellm_metadata"),
        Some(&applied_bucket(&["cs-aidr"], "a"))
    );
    assert_eq!(seen.get("metadata"), Some(&applied_bucket(&[], "b")));
    let mut other = BTreeMap::from([
        ("messages".to_owned(), GuardrailValue::Array(Vec::new())),
        ("metadata".to_owned(), applied_bucket(&["other"], "c")),
    ]);
    guardrail
        .apply_guardrail(inputs(), &mut other, GuardrailInputType::Request, None)
        .await
        .expect("the call succeeds");
    assert_eq!(
        other.get("metadata"),
        Some(&applied_bucket(&["other", "cs-aidr"], "c"))
    );
}

#[tokio::test]
async fn apply_guardrail_carries_non_empty_tools_and_tool_calls() {
    let (base, _captured, _server) = serve(
        StatusCode::OK,
        r#"{"result": {"transformed": true, "guard_output": {"messages": [{"role": "user", "content": "masked"}]}}}"#,
    )
    .await;
    let guardrail = handler(&base, None);
    let tools = vec![GuardrailToolParam {
        tool_type: "function".to_owned(),
        extra: BTreeMap::new(),
    }];
    let calls = vec![GuardrailValue::Text("call".to_owned())];
    let mut request_data = BTreeMap::new();
    let kept = guardrail
        .apply_guardrail(
            GuardrailApiInputs {
                tools: Some(tools.clone()),
                tool_calls: Some(calls.clone()),
                ..inputs()
            },
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("the call succeeds");
    assert_eq!(kept.tools, Some(tools));
    assert_eq!(kept.tool_calls, Some(calls));
    let dropped = guardrail
        .apply_guardrail(
            GuardrailApiInputs {
                tools: Some(Vec::new()),
                tool_calls: Some(Vec::new()),
                ..inputs()
            },
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("the call succeeds");
    assert_eq!(dropped.tools, None);
    assert_eq!(dropped.tool_calls, None);
}

#[tokio::test]
async fn apply_guardrail_maps_failures_to_http_errors() {
    let (blocked, _captured, _first) =
        serve(StatusCode::OK, r#"{"result": {"blocked": true}}"#).await;
    let mut request_data = BTreeMap::new();
    let failure = handler(&blocked, None)
        .apply_guardrail(
            inputs(),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("a block fails the call");
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("a block is an http object");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("guardrail_name"),
        Some(&GuardrailValue::Text("cs-aidr".to_owned()))
    );
    let (malformed, _captured, _second) = serve(
        StatusCode::OK,
        r#"{"result": {"transformed": true, "guard_output": "nope"}}"#,
    )
    .await;
    let failure = handler(&malformed, None)
        .apply_guardrail(
            inputs(),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("a malformed transformed reply fails the call");
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("a malformed transformed reply is an http object");
    };
    assert_eq!(status, 500);
    assert_eq!(detail.get("guardrail_name"), Some(&GuardrailValue::Null));
    let (broken, _captured, _third) = serve(StatusCode::INTERNAL_SERVER_ERROR, "oops").await;
    let failure = handler(&broken, None)
        .apply_guardrail(
            inputs(),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("a server error fails the closed call");
    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("a server error is a plain http failure");
    };
    assert_eq!(status, 500);
    assert!(detail.contains("status 500"));
}
