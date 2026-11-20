use std::collections::{BTreeMap, VecDeque};
use std::io::Error;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::rag::gemini_ingestion::port::{
    GeminiReply, GeminiTransport, TransportError, TransportRequest,
};
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const HEADERS: [&str; 7] = [
    "content-type",
    "x-goog-api-key",
    "x-goog-upload-protocol",
    "x-goog-upload-command",
    "x-goog-upload-header-content-length",
    "x-goog-upload-header-content-type",
    "x-goog-upload-offset",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum Step {
    Reply {
        status: u16,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        body: String,
    },
    Failure {
        failure: String,
    },
}

pub(super) struct Scripted {
    steps: Mutex<VecDeque<Step>>,
    pub(super) sent: Mutex<Vec<Value>>,
}

impl Scripted {
    pub(super) fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: Mutex::new(VecDeque::from(steps)),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn answer(&self) -> Result<GeminiReply, TransportError> {
        let next = self
            .steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match next {
            Some(Step::Reply {
                status,
                headers,
                body,
            }) => Ok(GeminiReply {
                status,
                headers: header_map(&headers)?,
                body: body.into_bytes(),
            }),
            Some(Step::Failure { failure }) => Err(failure_error(&failure)),
            None => Err(failure_error("the helper script has no reply left")),
        }
    }
}

#[async_trait]
impl GeminiTransport for Scripted {
    async fn send(&self, request: TransportRequest) -> Result<GeminiReply, TransportError> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(project(request)?);
        self.answer()
    }
}

fn failure_error(message: &str) -> TransportError {
    TransportError {
        source: Box::new(Error::other(message.to_owned())),
    }
}

fn header_map(pairs: &BTreeMap<String, String>) -> Result<HeaderMap, TransportError> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| failure_error(&error.to_string()))?;
        let header_value =
            HeaderValue::from_str(value).map_err(|error| failure_error(&error.to_string()))?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

fn project(request: TransportRequest) -> Result<Value, TransportError> {
    let body = request
        .body
        .into_bytes()
        .map_err(|error| failure_error(&error.to_string()))?;
    let mut shown = Map::new();
    for name in HEADERS {
        let Some(value) = request.headers.get(name) else {
            continue;
        };
        let masked = if name == "x-goog-api-key" {
            "<present>".to_owned()
        } else {
            value.to_str().unwrap_or_default().to_owned()
        };
        shown.insert(name.to_owned(), Value::String(masked));
    }
    let decoded = serde_json::from_slice::<Value>(&body).ok();
    let raw_bytes = decoded.is_none().then(|| Value::from(body.len()));
    Ok(json!({
        "method": request.method.as_str(),
        "url": request.url,
        "headers": Value::Object(shown),
        "body": decoded.unwrap_or(Value::Null),
        "raw_body_bytes": raw_bytes.unwrap_or(Value::Null),
        "timeout_seconds": request.timeout.as_secs(),
    }))
}
