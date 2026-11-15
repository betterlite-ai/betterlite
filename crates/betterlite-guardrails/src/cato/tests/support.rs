use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io::Error as IoError;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::{body, list, object, text};
use crate::cato::{CatoGuardrail, CatoParams};

pub(super) type Captured = Arc<Mutex<Vec<(BTreeMap<String, String>, String)>>>;
pub(super) type Server = JoinSet<Result<(), IoError>>;

#[derive(Debug, Clone, Serialize)]
pub(super) struct Action {
    pub(super) action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) detection_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Analysis {
    pub(super) policy_drill_down: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Redacted {
    pub(super) all_redacted_messages: Vec<GuardrailValue>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Answer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) required_action: Option<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) analysis_result: Option<Analysis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) redacted_chat: Option<Redacted>,
}

pub(super) fn no_action() -> Answer {
    Answer {
        required_action: None,
        analysis_result: None,
        redacted_chat: None,
    }
}

pub(super) fn action(action_type: &str, detection_message: Option<&str>) -> Answer {
    Answer {
        required_action: Some(Action {
            action_type: action_type.to_owned(),
            detection_message: detection_message.map(str::to_owned),
        }),
        analysis_result: None,
        redacted_chat: None,
    }
}

// an anonymize answer whose redacted messages are given in inspection order
pub(super) fn anonymized(messages: &[GuardrailValue]) -> Answer {
    let mut answer = action("anonymize_action", None);
    answer.redacted_chat = Some(Redacted {
        all_redacted_messages: messages.to_vec(),
    });
    answer
}

pub(super) async fn serve(status: StatusCode, answer: Answer) -> (String, Server, Captured) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let router = Router::new()
        .route("/fw/v1/analyze", post(handle))
        .with_state((Arc::clone(&captured), status, answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server, captured)
}

pub(super) async fn serve_raw(raw: String) -> (String, Server) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let router = Router::new().route(
        "/fw/v1/analyze",
        post(move || {
            let payload = raw.clone();
            async move { ([(CONTENT_TYPE, "application/json")], payload) }
        }),
    );
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

pub(super) fn raw_answer_of_size(size: usize) -> String {
    format!(
        "{{\"padding\":\"{}\"}}",
        "a".repeat(size.saturating_sub(14))
    )
}

pub(super) fn assert_header(headers: &BTreeMap<String, String>, name: &str, value: &str) {
    assert_eq!(headers.get(name).map(String::as_str), Some(value));
}

async fn handle(
    State((captured, status, answer)): State<(Captured, StatusCode, Answer)>,
    headers: HeaderMap,
    request_body: String,
) -> (StatusCode, Json<Answer>) {
    let mut recorded = BTreeMap::new();
    for (name, value) in &headers {
        if let Ok(header_value) = value.to_str() {
            recorded.insert(name.as_str().to_owned(), header_value.to_owned());
        }
    }
    captured
        .lock()
        .expect("captured")
        .push((recorded, request_body));
    (status, Json(answer))
}

#[derive(Debug, Deserialize)]
pub(super) struct PostedBody {
    pub(super) messages: Vec<GuardrailValue>,
}

pub(super) fn build(base: &str) -> CatoGuardrail {
    CatoGuardrail::new(
        CatoParams {
            guardrail_name: Some("cato".to_owned()),
            api_key: Some(SecretString::from("test-key")),
            api_base: Some(base.to_owned()),
            inspect_embeddings: None,
            ssl_verify: None,
            timeout: None,
        },
        &BTreeMap::<String, String>::new(),
    )
    .expect("guardrail")
}

pub(super) fn chat_data() -> BTreeMap<String, GuardrailValue> {
    body(&[
        ("litellm_call_id", text("call-1")),
        (
            "messages",
            list(&[object(&[
                ("content", text("hello")),
                ("role", text("user")),
            ])]),
        ),
    ])
}

// one chat response with the given message body
pub(super) fn chat_response(message: &[(&str, GuardrailValue)]) -> GuardrailValue {
    object(&[("choices", list(&[object(&[("message", object(message))])]))])
}
