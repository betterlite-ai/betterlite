use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;

use async_trait::async_trait;
use http::HeaderMap;
use thiserror::Error;
use tokio::time::sleep;

use super::installation::Installation;
use super::payload::{MOCK_LOGGING_PAYLOAD, MOCK_STORAGE_OBJECT};
use super::request::{GcsMethod, GcsRequest};
use super::settings::{GcsMockConfigError, GcsMockSettings};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcsHttpResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub url: String,
    pub elapsed: Duration,
}

#[async_trait]
pub trait GcsHttpClient: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn send(&self, request: GcsRequest) -> Result<GcsHttpResponse, Self::Error>;
}

#[derive(Debug, Error)]
pub enum GcsMockError<TransportError>
where
    TransportError: Error + 'static,
{
    #[error("no original HTTP handler is installed for {method}")]
    MissingOriginal { method: GcsMethod },
    #[error("the original HTTP handler failed")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the mock response body could not be serialized")]
    Payload {
        #[source]
        source: serde_json::Error,
    },
}

pub struct GcsMockClient<Real> {
    pub settings: GcsMockSettings,
    pub real: Option<Real>,
    pub mocks_initialized: bool,
}

impl<Real> GcsMockClient<Real> {
    pub const fn install(&mut self) -> Installation {
        if self.mocks_initialized {
            return Installation::AlreadyInstalled;
        }
        self.mocks_initialized = true;
        Installation::Installed
    }
}

pub fn create_mock_gcs_client<Real>(
    environment: &BTreeMap<String, String>,
    real: Option<Real>,
) -> Result<GcsMockClient<Real>, GcsMockConfigError> {
    let settings = GcsMockSettings::from_environment(environment)?;
    Ok(GcsMockClient {
        settings,
        real,
        mocks_initialized: settings.enabled,
    })
}

#[async_trait]
impl<Real> GcsHttpClient for GcsMockClient<Real>
where
    Real: GcsHttpClient,
{
    type Error = GcsMockError<Real::Error>;

    async fn send(&self, request: GcsRequest) -> Result<GcsHttpResponse, Self::Error> {
        if self.mocks_initialized && request.targets_gcs() {
            sleep(self.settings.latency).await;
            return mock_response(&request, self.settings.latency)
                .map_err(|source| GcsMockError::Payload { source });
        }
        let Some(real) = self.real.as_ref() else {
            return Err(GcsMockError::MissingOriginal {
                method: request.method(),
            });
        };
        real.send(request)
            .await
            .map_err(|source| GcsMockError::Transport { source })
    }
}

fn mock_response(
    request: &GcsRequest,
    latency: Duration,
) -> Result<GcsHttpResponse, serde_json::Error> {
    let (status, body) = match request {
        GcsRequest::Get(_) => (200, serde_json::to_vec(&MOCK_LOGGING_PAYLOAD)?),
        GcsRequest::Post(_) => (200, serde_json::to_vec(&MOCK_STORAGE_OBJECT)?),
        GcsRequest::Delete(_) => (204, Vec::new()),
    };
    Ok(GcsHttpResponse {
        status,
        headers: HeaderMap::new(),
        body,
        url: request.url().to_owned(),
        elapsed: latency,
    })
}
