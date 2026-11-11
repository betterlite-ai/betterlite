use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

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
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::{HelperError, OrHelperError as _};

use super::input::Reply;

const REPORTED_HEADERS: [&str; 5] = [
    "content-type",
    "authorization",
    "x-amz-date",
    "x-amz-security-token",
    "x-amz-content-sha256",
];

#[derive(Clone)]
pub(super) struct Sent {
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    deadline: Duration,
}

#[derive(Clone)]
pub(super) struct Script {
    reply: Arc<Mutex<Option<Reply>>>,
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl Script {
    pub(super) fn new(reply: &Reply) -> Self {
        Self {
            reply: Arc::new(Mutex::new(Some(reply.clone()))),
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn observed(&self) -> Result<Option<Value>, Box<HelperError>> {
        let first = self
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned();
        let Some(request) = first else {
            return Ok(None);
        };
        Ok(Some(json!({
            "method": request.method.as_str(),
            "url": request.url,
            "body": String::from_utf8(request.body).or_value_error()?,
            "timeout": request.deadline.as_secs_f64(),
            "headers": reported_headers(&request.headers),
        })))
    }

    fn answer(&self) -> Result<UpstreamResponse, TransportError> {
        let scripted = self
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(reply) = scripted else {
            return Err(TransportError::Closed);
        };
        if reply.failure {
            return Err(TransportError::Closed);
        }
        let pairs = reply.headers.into_iter().collect::<Vec<(String, String)>>();
        Ok(UpstreamResponse {
            status: reply.status,
            headers: header_map(&pairs),
            body: reply.body.into_bytes(),
        })
    }
}

impl CountTokensTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        record(&self.sent, request);
        ready(self.answer())
    }
}

fn record(sent: &Mutex<Vec<Sent>>, request: OutboundRequest) {
    let payload = match request.body {
        RequestBody::Json(bytes) => bytes,
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
            Vec::new()
        }
    };
    let pairs = request
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    sent.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Sent {
            method: request.method,
            url: request.url.to_string(),
            headers: pairs,
            body: payload,
            deadline: request.deadline,
        });
}

fn reported_headers(headers: &[(String, String)]) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter(|(name, _)| REPORTED_HEADERS.contains(&name.to_ascii_lowercase().as_str()))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect()
}

fn header_map(pairs: &[(String, String)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let Ok(header_name) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(header_value) = HeaderValue::from_str(value) else {
            continue;
        };
        headers.insert(header_name, header_value);
    }
    headers
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
        ready(Ok::<Credentials, CredentialError>(credentials))
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
