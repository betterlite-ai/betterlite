use std::fmt::Display;

use betterlite_oracle_helper_api::error::HelperError;
use http::{HeaderMap, header::CONTENT_TYPE};
use serde_json::{Map, Value, json};

use super::case::{ORACLE_UPSTREAM, Operation, PLACEHOLDER};

pub(super) type Failure = Box<HelperError>;

const SIGNED: &str = "<signed>";
const SIGNATURE_HEADERS: [&str; 3] = ["authorization", "x-amz-date", "x-amz-security-token"];

pub(super) fn failure(status: Option<u16>, error: &dyn Display) -> Failure {
    Box::new(HelperError {
        class: String::from("HelperError"),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

// one request as the recording transport keeps it
#[derive(Clone)]
pub(super) struct Sent {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Option<Vec<u8>>,
}

fn body_view(sent: &Sent) -> Value {
    let Some(bytes) = sent.body.as_deref().filter(|bytes| !bytes.is_empty()) else {
        return Value::Null;
    };
    let content_type = sent
        .headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if content_type.contains("json") {
        return serde_json::from_slice(bytes).unwrap_or(Value::Null);
    }
    Value::String(String::from_utf8_lossy(bytes).into_owned())
}

fn header_view(sent: &Sent, op: Operation) -> Value {
    let mut headers = Map::new();
    let signed = matches!(op, Operation::SagemakerChat | Operation::SagemakerNova);
    for (name, value) in &sent.headers {
        let lowered = name.as_str();
        if !op.headers().contains(&lowered) {
            continue;
        }
        let Ok(raw) = value.to_str() else {
            continue;
        };
        let text = if signed && SIGNATURE_HEADERS.contains(&lowered) {
            SIGNED
        } else {
            raw
        };
        headers.insert(lowered.to_owned(), Value::String(text.to_owned()));
    }
    Value::Object(headers)
}

fn request_view(sent: &Sent, op: Operation) -> Value {
    json!({
        "method": sent.method,
        "url": sent.url.replace(ORACLE_UPSTREAM, PLACEHOLDER),
        "headers": header_view(sent, op),
        "body": body_view(sent),
    })
}

pub(super) fn answer(sent: &[Sent], op: Operation) -> Value {
    let requests: Vec<Value> = sent
        .iter()
        .map(|request| request_view(request, op))
        .collect();
    json!({"requests": requests})
}
