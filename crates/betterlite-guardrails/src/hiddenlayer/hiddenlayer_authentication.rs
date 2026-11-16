use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use url::Url;

pub mod detection;
pub mod error;
pub mod settings;
mod tests;
pub mod token;

use error::HiddenlayerError;
use settings::{DEFAULT_AUTH_TIMEOUT, HiddenlayerCredentials, HiddenlayerSettings};
use token::JwtTransport;

#[must_use]
pub fn is_saas(api_base: &str) -> bool {
    Url::parse(api_base)
        .ok()
        .and_then(|parsed| parsed.host_str().map(ToOwned::to_owned))
        .is_some_and(|host| host.ends_with("hiddenlayer.ai"))
}

// one shared authentication primitive: both the v1 and the v2 guardrail construct it identically
pub struct HiddenlayerAuth {
    settings: HiddenlayerSettings,
    client: reqwest::Client,
    jwt: Mutex<Option<SecretString>>,
    transport: Arc<dyn JwtTransport>,
}

impl HiddenlayerAuth {
    pub fn new<L>(
        credentials: &HiddenlayerCredentials,
        lookup: &L,
        client: reqwest::Client,
        transport: Arc<dyn JwtTransport>,
    ) -> Result<Self, HiddenlayerError>
    where
        L: EnvironmentLookup,
    {
        let settings = settings::resolve(credentials, lookup)?;
        let initial = if is_saas(&settings.api_base) {
            let request = settings.token_request()?;
            Some(SecretString::from(transport.fetch_token(&request)?))
        } else {
            None
        };
        Ok(Self {
            settings,
            client,
            jwt: Mutex::new(initial),
            transport,
        })
    }

    pub async fn post_detection(
        &self,
        request: &detection::DetectionRequest<'_>,
    ) -> Result<detection::DetectionResponse, HiddenlayerError> {
        self.post_detection_with_timeout(request, DEFAULT_AUTH_TIMEOUT)
            .await
    }

    // the guardrails pass their configured timeout here, where LiteLLM hands it to the httpx request
    pub async fn post_detection_with_timeout(
        &self,
        request: &detection::DetectionRequest<'_>,
        timeout: Duration,
    ) -> Result<detection::DetectionResponse, HiddenlayerError> {
        let first = self.send_once(request, timeout).await?;
        // an empty stored token is still "present" for the 401 refresh check, the way a Python "" is not None
        let has_token = self
            .jwt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        if first.status().as_u16() != 401 || !has_token {
            return detection::settle(first).await;
        }
        self.refresh().await?;
        let retried = self.send_once(request, timeout).await?;
        detection::settle(retried).await
    }

    async fn refresh(&self) -> Result<(), HiddenlayerError> {
        let request = self.settings.token_request()?;
        let transport = Arc::clone(&self.transport);
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(move || transport.fetch_token(&request));
        let token = match tasks.join_next().await {
            Some(joined) => joined.map_err(|source| HiddenlayerError::RefreshTask { source })?,
            None => return Ok(()),
        };
        *self.jwt.lock().unwrap_or_else(PoisonError::into_inner) = Some(SecretString::from(token?));
        Ok(())
    }

    async fn send_once(
        &self,
        request: &detection::DetectionRequest<'_>,
        timeout: Duration,
    ) -> Result<reqwest::Response, HiddenlayerError> {
        let url = format!("{}/{}", self.settings.api_base, request.path);
        let mut builder = self
            .client
            .post(url)
            .timeout(timeout)
            .headers(request.headers.clone())
            .json(request.body);
        // the Authorization header is only sent for a token with characters, since an empty one is falsy
        let token = self
            .jwt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|token| !token.expose_secret().is_empty())
            .cloned();
        if let Some(present) = token {
            builder = builder.bearer_auth(present.expose_secret());
        }
        builder
            .send()
            .await
            .map_err(|source| HiddenlayerError::Transport { source })
    }
}
