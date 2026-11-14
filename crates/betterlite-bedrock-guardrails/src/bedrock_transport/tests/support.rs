#![cfg(test)]

use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::sections::content_filter_records::ConfidenceThreshold;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::bedrock_credentials::CredentialSource;
use crate::bedrock_post::{GuardrailTransport, TransportLog};
use crate::bedrock_transport::{BedrockGuardrail, BedrockGuardrailInit};

pub(crate) fn guardrail_init() -> BedrockGuardrailInit {
    BedrockGuardrailInit {
        guardrail_identifier: Some("gr-1".to_owned()),
        guardrail_version: Some("1".to_owned()),
        disable_exception_on_block: None,
        checks: None,
        content_filter_threshold: Some(ConfidenceThreshold(0.5)),
        prompt_attack_threshold: Some(ConfidenceThreshold(0.5)),
        pii_confidence_threshold: Some(ConfidenceThreshold(0.5)),
        chunk_budget_chars: 25_000,
        contextual_grounding: false,
        latest_role_message_only: false,
        mask_request_content: false,
        mask_response_content: false,
        buffer_until_moderated: None,
        streaming_sampling_rate: None,
        streaming_end_of_stream_only: None,
        buffer_release_on_scan: None,
        optional_params: OpaqueParams::default(),
        timeout: None,
    }
}

pub(crate) fn guardrail_with(optional_params: OpaqueParams) -> BedrockGuardrail {
    let mut init = guardrail_init();
    init.optional_params = optional_params;
    BedrockGuardrail::__init__(init).expect("the test guardrail")
}

pub(crate) fn aws_credentials() -> Credentials {
    Credentials {
        access_key_id: SecretString::from("AKIAIOSFODNN7EXAMPLE".to_owned()),
        secret_access_key: SecretString::from(
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_owned(),
        ),
        session_token: None,
    }
}

pub(crate) fn prepared_request() -> AwsSignedRequest {
    AwsSignedRequest {
        method: "POST".to_owned(),
        url: "https://bedrock-runtime.us-west-2.amazonaws.com/guardrail/gr-1/version/1/apply"
            .to_owned(),
        headers: vec![
            ("Content-Type".to_owned(), "application/json".to_owned()),
            (
                "Authorization".to_owned(),
                "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20261113/us-west-2/bedrock/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=abc".to_owned(),
            ),
        ],
        body: br#"{"source":"INPUT","content":[]}"#.to_vec(),
    }
}

pub(crate) fn signing_time() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_764_000_000)
}

#[derive(Clone, Default)]
pub(crate) struct RecordingCredentials {
    pub(crate) calls: Arc<Mutex<Vec<(AwsSettings, String)>>>,
    answer: Arc<Mutex<Option<Result<Credentials, CredentialError>>>>,
}

impl RecordingCredentials {
    pub(crate) fn replying(credentials: Credentials) -> Self {
        Self {
            calls: Arc::default(),
            answer: Arc::new(Mutex::new(Some(Ok(credentials)))),
        }
    }

    pub(crate) fn failing(error: CredentialError) -> Self {
        Self {
            calls: Arc::default(),
            answer: Arc::new(Mutex::new(Some(Err(error)))),
        }
    }

    pub(crate) fn calls(&self) -> Vec<(AwsSettings, String)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CredentialSource for RecordingCredentials {
    fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((settings, region));
        let answer = self
            .answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or(Err(CredentialError::MissingCredentials));
        ready(answer)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Sent {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    pub(crate) deadline: Duration,
    pub(crate) sensitive_headers: Vec<String>,
}

#[derive(Clone, Default)]
pub(crate) struct RecordingTransport {
    sent: Arc<Mutex<Vec<Sent>>>,
    reply: Arc<Mutex<Option<Result<UpstreamResponse, TransportError>>>>,
}

impl RecordingTransport {
    pub(crate) fn replying(status: u16, body: &str) -> Self {
        let response = UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
        };
        Self {
            sent: Arc::default(),
            reply: Arc::new(Mutex::new(Some(Ok(response)))),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            sent: Arc::default(),
            reply: Arc::new(Mutex::new(Some(Err(TransportError::Closed)))),
        }
    }

    pub(crate) fn sent(&self) -> Vec<Sent> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl GuardrailTransport for RecordingTransport {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let body = match request.body {
            RequestBody::Json(bytes) => bytes,
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Vec::new()
            }
        };
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        let sensitive_headers = request
            .headers
            .iter()
            .filter(|(_, value)| value.is_sensitive())
            .map(|(name, _)| name.to_string())
            .collect();
        let sent = Sent {
            method: request.method.to_string(),
            url: request.url.to_string(),
            headers,
            body,
            deadline: request.deadline,
            sensitive_headers,
        };
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(sent);
        let reply = self
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or(Err(TransportError::Closed));
        ready(reply)
    }
}

#[derive(Clone, Default)]
pub(crate) struct RecordingLog {
    details: Arc<Mutex<Vec<String>>>,
}

impl RecordingLog {
    pub(crate) fn details(&self) -> Vec<String> {
        self.details
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TransportLog for RecordingLog {
    fn guardrail_failed_to_respond(&self, detail: &str) {
        self.details
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(detail.to_owned());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) level: Level,
    pub(crate) message: String,
}

#[derive(Clone, Default)]
pub(crate) struct Captured {
    entries: Arc<Mutex<Vec<Entry>>>,
}

impl Captured {
    pub(crate) fn entries(&self) -> Vec<Entry> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn warnings(&self) -> Vec<String> {
        self.messages(Level::WARN)
    }

    pub(crate) fn errors(&self) -> Vec<String> {
        self.messages(Level::ERROR)
    }

    fn messages(&self, level: Level) -> Vec<String> {
        self.entries()
            .into_iter()
            .filter(|entry| entry.level == level)
            .map(|entry| entry.message)
            .collect()
    }
}

struct CaptureLayer(Arc<Mutex<Vec<Entry>>>);

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        let entry = Entry {
            level: *event.metadata().level(),
            message: visitor.0,
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }
}

struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

pub(crate) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let layer = CaptureLayer(Arc::clone(&captured.entries));
    let guard = set_default(registry().with(layer));
    (captured, guard)
}
