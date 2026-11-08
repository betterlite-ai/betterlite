use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use http::HeaderMap;

use crate::gcs_mock::auth::{
    AccessToken, EnsureAccessToken, GetTokenAndUrl, TokenAndUrl, VertexAuth,
};
use crate::gcs_mock::client::{GcsHttpClient, GcsHttpResponse};
use crate::gcs_mock::request::{GcsReadRequest, GcsRequest, GcsWriteRequest};

pub(crate) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn read_request(url: &str) -> GcsRequest {
    GcsRequest::Get(GcsReadRequest {
        url: url.to_owned(),
        params: BTreeMap::from([("alt".to_owned(), "media".to_owned())]),
        headers: HeaderMap::from_iter([(
            http::HeaderName::from_static("authorization"),
            http::HeaderValue::from_static("Bearer mock-gcs-token"),
        )]),
        follow_redirects: Some(false),
    })
}

pub(crate) fn write_request(url: &str) -> GcsWriteRequest {
    GcsWriteRequest {
        url: url.to_owned(),
        data: Some(Json::Text("{\"kind\":\"storage#object\"}".to_owned())),
        json: None,
        params: BTreeMap::from([("uploadType".to_owned(), "media".to_owned())]),
        headers: HeaderMap::from_iter([(
            http::HeaderName::from_static("content-type"),
            http::HeaderValue::from_static("application/json"),
        )]),
        timeout: Some(Duration::from_secs(30)),
        stream: false,
        content: None,
    }
}

pub(crate) struct RecordingClient {
    pub(crate) sent: Arc<Mutex<Vec<GcsRequest>>>,
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
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

#[derive(Debug, thiserror::Error)]
#[error("the test transport failed")]
pub(crate) struct TestTransportError;

pub(crate) struct FailingClient;

#[async_trait]
impl GcsHttpClient for FailingClient {
    type Error = TestTransportError;

    async fn send(&self, _request: GcsRequest) -> Result<GcsHttpResponse, TestTransportError> {
        Err(TestTransportError)
    }
}

pub(crate) struct FakeAuth {
    pub(crate) calls: AtomicUsize,
}

impl FakeAuth {
    pub(crate) fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl VertexAuth for FakeAuth {
    type Error = Infallible;

    async fn ensure_access_token_async(
        &self,
        _request: EnsureAccessToken,
    ) -> Result<AccessToken, Infallible> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(AccessToken {
            token: "original-token".to_owned(),
            project_id: "original-project".to_owned(),
        })
    }

    fn ensure_access_token(&self, _request: EnsureAccessToken) -> Result<AccessToken, Infallible> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(AccessToken {
            token: "original-token".to_owned(),
            project_id: "original-project".to_owned(),
        })
    }

    fn get_token_and_url(&self, _request: GetTokenAndUrl) -> Result<TokenAndUrl, Infallible> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(TokenAndUrl {
            token: "original-token".to_owned(),
            url: "https://original.example.com".to_owned(),
        })
    }
}

pub(crate) fn ensure_request() -> EnsureAccessToken {
    EnsureAccessToken {
        credentials: None,
        project_id: None,
        custom_llm_provider: "vertex_ai".to_owned(),
    }
}

pub(crate) fn token_request() -> GetTokenAndUrl {
    GetTokenAndUrl {
        model: "gcs-bucket".to_owned(),
        auth_header: None,
        vertex_credentials: None,
        vertex_project: None,
        vertex_location: None,
        gemini_api_key: None,
        stream: None,
        custom_llm_provider: "vertex_ai".to_owned(),
        api_base: None,
    }
}
