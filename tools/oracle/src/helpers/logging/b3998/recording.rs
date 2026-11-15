use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::gcs_client::gcs_storage_auth::STORAGE_ENDPOINT;
use betterlite_logging::gcs_logger::runtime::IdSource;
use betterlite_logging::gcs_mock::auth::{
    AccessToken, EnsureAccessToken, GetTokenAndUrl, MOCK_TOKEN, TokenAndUrl, VertexAuth,
};
use betterlite_logging::gcs_mock::client::{GcsHttpClient, GcsHttpResponse};
use betterlite_logging::gcs_mock::request::GcsRequest;
use betterlite_logging::prometheus_query::clock::Clock;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::input::Reply;

// the clock and the ids stand in for the values LiteLLM reads from the process, so the URLs can be compared
pub(super) const FIXED_NOW: i64 = 1_715_947_200;
pub(super) const FIXED_UUID: &str = "0123456789abcdef0123456789abcdef";

#[derive(Debug, thiserror::Error)]
#[error("the oracle authentication failed")]
pub(super) struct AuthFailure;

#[derive(Debug, thiserror::Error)]
#[error("the oracle transport failed")]
pub(super) struct TransportFailure;

// LiteLLM's gcs_bucket_mock_client answers with this token and the storage host
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct FixedAuth;

#[async_trait]
impl VertexAuth for FixedAuth {
    type Error = AuthFailure;

    async fn ensure_access_token_async(
        &self,
        request: EnsureAccessToken,
    ) -> Result<AccessToken, AuthFailure> {
        Ok(access(request))
    }

    fn ensure_access_token(&self, request: EnsureAccessToken) -> Result<AccessToken, AuthFailure> {
        Ok(access(request))
    }

    fn get_token_and_url(&self, _request: GetTokenAndUrl) -> Result<TokenAndUrl, AuthFailure> {
        Ok(TokenAndUrl {
            token: SecretString::from(MOCK_TOKEN),
            url: STORAGE_ENDPOINT.to_owned(),
        })
    }
}

fn access(request: EnsureAccessToken) -> AccessToken {
    AccessToken {
        token: SecretString::from(MOCK_TOKEN),
        project_id: request.project_id.unwrap_or_default(),
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(FIXED_NOW, 0).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct FixedIds;

impl IdSource for FixedIds {
    fn uuid_hex(&self) -> String {
        FIXED_UUID.to_owned()
    }
}

#[derive(Clone, Debug)]
pub(super) struct RecordingClient {
    pub(super) sent: Arc<Mutex<Vec<GcsRequest>>>,
    replies: Arc<Mutex<VecDeque<Reply>>>,
}

impl RecordingClient {
    pub(super) fn new(replies: &[Reply]) -> Self {
        Self {
            sent: Arc::default(),
            replies: Arc::new(Mutex::new(replies.iter().cloned().collect())),
        }
    }
}

#[async_trait]
impl GcsHttpClient for RecordingClient {
    type Error = TransportFailure;

    async fn send(&self, request: GcsRequest) -> Result<GcsHttpResponse, TransportFailure> {
        let url = request.url().to_owned();
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_default();
        Ok(GcsHttpResponse {
            status: reply.status,
            headers: HeaderMap::new(),
            body: reply.body.into_bytes(),
            url,
            elapsed: Duration::ZERO,
        })
    }
}

pub(super) fn uploads(client: &RecordingClient) -> usize {
    let sent = client.sent.lock().unwrap_or_else(PoisonError::into_inner);
    sent.iter()
        .filter(|request| matches!(request, GcsRequest::Post(_)))
        .count()
}

pub(super) fn requests_json(client: &RecordingClient) -> Result<Value, regex::Error> {
    let sent = client.sent.lock().unwrap_or_else(PoisonError::into_inner);
    let entries = sent
        .iter()
        .map(request_json)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(entries))
}

fn request_json(request: &GcsRequest) -> Result<Value, regex::Error> {
    let headers = match request {
        GcsRequest::Get(read) => &read.headers,
        GcsRequest::Post(write) | GcsRequest::Delete(write) => &write.headers,
    };
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    let (url, body) = match request {
        GcsRequest::Post(write) => (mask(request.url())?, body_lines(write.data.as_ref())),
        GcsRequest::Get(_) | GcsRequest::Delete(_) => (request.url().to_owned(), Value::Null),
    };
    Ok(json!({
        "method": request.method().to_string(),
        "url": url,
        "authorization": authorization,
        "body": body,
    }))
}

fn body_lines(data: Option<&Json>) -> Value {
    let Some(Json::Text(text)) = data else {
        return Value::Null;
    };
    Value::Array(
        text.split('\n')
            .map(|line| serde_json::from_str::<Value>(line).unwrap_or(Value::Null))
            .collect(),
    )
}

// the date, the batch id and the uuid differ between the two languages, so only those parts of a POST URL are masked
pub(super) fn mask(url: &str) -> Result<String, regex::Error> {
    let pattern = regex::Regex::new(r"\d{13}-[0-9a-f]{8}|\d{4}-\d{2}-\d{2}|[0-9a-f]{32}")?;
    Ok(pattern
        .replace_all(url, |found: &regex::Captures<'_>| {
            match found.get(0).map_or(0, |span| span.as_str().len()) {
                10 => "{date}",
                22 => "{batch}",
                _ => "{uuid}",
            }
        })
        .into_owned())
}
