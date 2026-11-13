#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use axum::Router;
use axum::body::Bytes;
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::call_or_fail_open;
use crate::crowdstrike_aidr::error::CrowdStrikeError;
use crate::crowdstrike_aidr::handler::{ConstructorArgs, CrowdStrikeAidr, Dependencies};
use crate::crowdstrike_aidr::models::{
    EventType, GuardInput, GuardPayload, Message, MessageContent,
};
use crate::crowdstrike_streaming::StreamingParams;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract as _;
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
    let request = inputs();
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
