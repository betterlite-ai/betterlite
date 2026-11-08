use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::gcs_mock::auth::{
    AccessToken, EnsureAccessToken, GetTokenAndUrl, TokenAndUrl, VertexAuth,
};
use betterlite_logging::gcs_mock::client::{GcsHttpClient, GcsHttpResponse};
use betterlite_logging::gcs_mock::request::{GcsRequest, GcsWriteRequest};
use http::HeaderMap;
use serde_json::{Map, Value, json};

pub(crate) struct RecordingClient {
    sent: Arc<Mutex<Vec<GcsRequest>>>,
    status: u16,
    body: Vec<u8>,
}

impl RecordingClient {
    pub(crate) fn new(sent: Arc<Mutex<Vec<GcsRequest>>>, status: u16, body: &str) -> Self {
        Self {
            sent,
            status,
            body: body.as_bytes().to_vec(),
        }
    }
}

#[async_trait]
impl GcsHttpClient for RecordingClient {
    type Error = Infallible;

    async fn send(&self, request: GcsRequest) -> Result<GcsHttpResponse, Infallible> {
        let url = request.url().to_owned();
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Ok(GcsHttpResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
            url,
            elapsed: Duration::ZERO,
        })
    }
}

pub(crate) struct FakeAuth;

#[async_trait]
impl VertexAuth for FakeAuth {
    type Error = Infallible;

    async fn ensure_access_token_async(
        &self,
        _request: EnsureAccessToken,
    ) -> Result<AccessToken, Infallible> {
        Ok(AccessToken {
            token: "original-token".to_owned(),
            project_id: "original-project".to_owned(),
        })
    }

    fn ensure_access_token(&self, _request: EnsureAccessToken) -> Result<AccessToken, Infallible> {
        Ok(AccessToken {
            token: "original-token".to_owned(),
            project_id: "original-project".to_owned(),
        })
    }

    fn get_token_and_url(&self, _request: GetTokenAndUrl) -> Result<TokenAndUrl, Infallible> {
        Ok(TokenAndUrl {
            token: "original-token".to_owned(),
            url: "https://original.example.com".to_owned(),
        })
    }
}

pub(crate) fn last_recorded(sent: &Arc<Mutex<Vec<GcsRequest>>>) -> Option<GcsRequest> {
    sent.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .last()
        .cloned()
}

pub(crate) fn observed_request(request: &GcsRequest) -> Value {
    match request {
        GcsRequest::Get(read) => json!({
            "method": "get",
            "url": read.url,
            "params": read.params,
            "headers": header_json(&read.headers),
            "follow_redirects": read.follow_redirects,
        }),
        GcsRequest::Post(write) => observed_write("post", write),
        GcsRequest::Delete(write) => observed_write("delete", write),
    }
}

fn observed_write(method: &str, write: &GcsWriteRequest) -> Value {
    json!({
        "method": method,
        "url": write.url,
        "data": write.data,
        "json": write.json,
        "params": write.params,
        "headers": header_json(&write.headers),
        "timeout_ms": write.timeout.and_then(|timeout| u64::try_from(timeout.as_millis()).ok()),
        "stream": write.stream,
        "content": write.content.as_ref().and_then(|bytes| String::from_utf8(bytes.clone()).ok()),
    })
}

fn header_json(headers: &HeaderMap) -> Value {
    Value::Object(
        headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_owned(), Value::String(text.to_owned())))
            })
            .collect::<Map<String, Value>>(),
    )
}

pub(crate) fn observed_body(body: &[u8]) -> Value {
    serde_json::from_slice(body).unwrap_or_else(|_error| {
        String::from_utf8(body.to_vec()).map_or(Value::Null, Value::String)
    })
}
