use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::gcs_mock::auth::{
    AccessToken, EnsureAccessToken, GetTokenAndUrl, TokenAndUrl, VertexAuth,
};
use betterlite_logging::gcs_mock::client::{GcsHttpClient, GcsHttpResponse};
use betterlite_logging::gcs_mock::request::GcsRequest;
use http::HeaderMap;
use secrecy::SecretString;

#[derive(Debug, thiserror::Error)]
#[error("the oracle authentication failed")]
pub(crate) struct AuthFailure;

#[derive(Debug, thiserror::Error)]
#[error("the oracle transport failed")]
pub(crate) struct TransportFailure;

#[derive(Clone, Default)]
pub(crate) struct FakeAuth {
    pub(crate) ensure_calls: Arc<Mutex<Vec<EnsureAccessToken>>>,
    pub(crate) token_calls: Arc<Mutex<Vec<GetTokenAndUrl>>>,
}

impl FakeAuth {
    fn record(&self, request: EnsureAccessToken) -> Result<AccessToken, AuthFailure> {
        self.ensure_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Ok(AccessToken {
            token: SecretString::from("mock-gcs-token"),
            project_id: "mock-project-id".to_owned(),
        })
    }
}

#[async_trait]
impl VertexAuth for FakeAuth {
    type Error = AuthFailure;

    async fn ensure_access_token_async(
        &self,
        request: EnsureAccessToken,
    ) -> Result<AccessToken, AuthFailure> {
        self.record(request)
    }

    fn ensure_access_token(&self, request: EnsureAccessToken) -> Result<AccessToken, AuthFailure> {
        self.record(request)
    }

    fn get_token_and_url(&self, request: GetTokenAndUrl) -> Result<TokenAndUrl, AuthFailure> {
        self.token_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Ok(TokenAndUrl {
            token: SecretString::from("mock-gcs-token"),
            url: "https://original.example.com".to_owned(),
        })
    }
}

pub(crate) struct RecordingClient {
    pub(crate) sent: Arc<Mutex<Vec<GcsRequest>>>,
    status: u16,
    body: Vec<u8>,
}

impl RecordingClient {
    pub(crate) fn new(sent: Arc<Mutex<Vec<GcsRequest>>>, status: u16, body: String) -> Self {
        Self {
            sent,
            status,
            body: body.into_bytes(),
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
        Ok(GcsHttpResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
            url,
            elapsed: Duration::ZERO,
        })
    }
}
