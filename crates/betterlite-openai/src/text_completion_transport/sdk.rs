use std::fmt;
use std::future::Future;
use std::pin::Pin;

use futures_util::Stream;
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::{Map, Value};
use thiserror::Error;

use crate::stream::SseError;

// The OpenAI SDK's two client classes; only construction differs for these endpoint calls
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Sync,
    Async,
}

// Which httpx client the handler hands the SDK constructor
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpClientSource {
    ClientSession,
    AClientSession,
    AsyncHttpClient,
}

#[derive(Debug, Clone)]
pub struct SdkArguments {
    pub api_key: Option<SecretString>,
    pub base_url: Option<String>,
    pub timeout: f64,
    pub max_retries: Option<i64>,
    pub organization: Option<String>,
    pub http_client: HttpClientSource,
}

// An APIStatusError's facts; header values may carry credentials, so Debug prints none of them
pub struct StatusError {
    pub status: Option<u16>,
    pub message: String,
    pub headers: HeaderMap,
    pub error_type: Option<String>,
    pub param: Option<String>,
    pub code: Option<String>,
}

impl fmt::Debug for StatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StatusError")
            .field("status", &self.status)
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

// What the OpenAI SDK raises: an API status error carries the response facts, Other is text an SDK implementation hands over.
#[derive(Debug, Error)]
pub enum SdkError {
    #[error("{}", .0.message)]
    Status(Box<StatusError>),
    #[error("the answer is not a completion: {0}")]
    Decode(#[source] serde_json::Error),
    #[error(transparent)]
    Sse(#[from] SseError),
    #[error("{message}")]
    Other { message: String },
}

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, SdkError>> + Send>>;

// A `completions.with_raw_response.create(**data)` answer before `.parse()` reads it
pub enum RawCompletion {
    Buffered {
        status: u16,
        headers: HeaderMap,
        body: Vec<u8>,
    },
    Stream {
        status: u16,
        headers: HeaderMap,
        body: ByteStream,
    },
}

// What `.parse()` returns: the Completion model or the SDK's chunk iterator
pub enum ParsedCompletion {
    Buffered(Box<super::reply::CompletionReply>),
    Stream(ByteStream),
}

// The SDK reads type, param and code from body["error"] when it is a mapping and from the body otherwise
fn error_fields(body: &[u8]) -> (Option<String>, Option<String>, Option<String>) {
    let Ok(Value::Object(mut fields)) = serde_json::from_slice::<Value>(body) else {
        return (None, None, None);
    };
    let data = match fields.remove("error") {
        Some(Value::Object(inner)) => inner,
        Some(_other) => return (None, None, None),
        None => fields,
    };
    let text = |name: &str| data.get(name).and_then(Value::as_str).map(str::to_owned);
    (text("type"), text("param"), text("code"))
}

#[must_use]
pub fn status_error(status: u16, headers: HeaderMap, body: &[u8]) -> SdkError {
    let (error_type, param, code) = error_fields(body);
    SdkError::Status(Box::new(StatusError {
        status: Some(status),
        message: format!(
            "Error code: {status} - {}",
            str::from_utf8(body).unwrap_or_default()
        ),
        headers,
        error_type,
        param,
        code,
    }))
}

impl RawCompletion {
    pub fn parse(self) -> Result<ParsedCompletion, SdkError> {
        match self {
            Self::Buffered {
                status,
                headers,
                body,
            } => {
                if status >= 400 {
                    return Err(status_error(status, headers, &body));
                }
                let reply = serde_json::from_slice(&body).map_err(SdkError::Decode)?;
                Ok(ParsedCompletion::Buffered(Box::new(reply)))
            }
            Self::Stream {
                status,
                headers,
                body,
            } => {
                if status >= 400 {
                    return Err(status_error(status, headers, &[]));
                }
                Ok(ParsedCompletion::Stream(body))
            }
        }
    }
}

// The message the OpenAI SDK raises when it has no API key to send
#[must_use]
pub fn missing_api_key() -> SdkError {
    SdkError::Other {
        message: "The api_key client option must be set either by passing api_key to the client or by setting the OPENAI_API_KEY environment variable".to_owned(),
    }
}

pub trait TextCompletionSdk: Send + Sync {
    fn kind(&self) -> ClientKind;

    fn create(
        &self,
        data: &Map<String, Value>,
    ) -> impl Future<Output = Result<RawCompletion, SdkError>> + Send;
}

pub trait ClientConstructor: Sync {
    type Client: TextCompletionSdk;

    // Ok(None) is an SDK that could not be built, such as one without any API key
    fn construct(
        &self,
        arguments: &SdkArguments,
        kind: ClientKind,
    ) -> Result<Option<Self::Client>, SdkError>;
}
