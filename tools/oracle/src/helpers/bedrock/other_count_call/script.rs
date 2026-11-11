use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::count_call::handler::{
    CountTokensCredentials, CountTokensHandler, CountTokensHandlerError, CountTokensHandlerRequest,
    CountTokensTransport,
};
use betterlite_bedrock::count_call::reply::CountResponse;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{HeaderName, HeaderValue};
use secrecy::SecretString;
use serde_json::{Map, Value, json};

use super::input::Reply;

const REPORTED_HEADERS: [&str; 5] = [
    "content-type",
    "authorization",
    "x-amz-date",
    "x-amz-security-token",
    "x-amz-content-sha256",
];

#[derive(Clone)]
pub(super) struct Script {
    reply: Reply,
    sent: Arc<Mutex<Option<Value>>>,
}

impl Script {
    pub(super) fn new(reply: &Reply) -> Self {
        Self {
            reply: reply.clone(),
            sent: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) fn observed(&self) -> Option<Value> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CountTokensTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(|| describe(&request));
        if self.reply.failure {
            return ready(Err(TransportError::Closed));
        }
        ready(Ok(UpstreamResponse {
            status: self.reply.status.unwrap_or(200),
            headers: self
                .reply
                .headers
                .iter()
                .filter_map(|(name, value)| {
                    Some((
                        HeaderName::from_bytes(name.as_bytes()).ok()?,
                        HeaderValue::from_str(value).ok()?,
                    ))
                })
                .collect(),
            body: self
                .reply
                .body
                .as_deref()
                .unwrap_or("{}")
                .as_bytes()
                .to_vec(),
        }))
    }
}

fn describe(request: &OutboundRequest) -> Value {
    let body = match &request.body {
        RequestBody::Json(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
            String::new()
        }
    };
    let headers: Map<String, Value> = request
        .headers
        .iter()
        .filter(|(name, _)| REPORTED_HEADERS.contains(&name.as_str()))
        .map(|(name, value)| {
            (
                name.to_string(),
                Value::from(value.to_str().unwrap_or_default()),
            )
        })
        .collect();
    json!({
        "method": request.method.as_str(),
        "url": request.url.as_str(),
        "body": body,
        "timeout": request.deadline.as_secs_f64(),
        "headers": headers,
    })
}

pub(super) struct FixtureCredentials {
    pub(super) resolved: Arc<AtomicUsize>,
}

impl CountTokensCredentials for FixtureCredentials {
    fn resolve(
        &self,
        settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.resolved.fetch_add(1, Ordering::Relaxed);
        let credentials = match (settings.access_key_id, settings.secret_access_key) {
            (Some(access_key_id), Some(secret_access_key)) => Credentials {
                access_key_id,
                secret_access_key,
                session_token: settings.session_token,
            },
            _ => Credentials {
                access_key_id: SecretString::from("AKIDEXAMPLE".to_owned()),
                secret_access_key: SecretString::from("fixture-secret-key".to_owned()),
                session_token: None,
            },
        };
        ready(Ok(credentials))
    }
}

pub(super) struct FixedHandler {
    pub(super) result: Option<CountResponse>,
}

impl CountTokensHandler for FixedHandler {
    fn handle_count_tokens_request(
        &self,
        _request: &CountTokensHandlerRequest<'_>,
        _now: SystemTime,
    ) -> impl Future<Output = Result<Option<CountResponse>, CountTokensHandlerError>> + Send {
        ready(Ok(self.result.clone()))
    }
}
