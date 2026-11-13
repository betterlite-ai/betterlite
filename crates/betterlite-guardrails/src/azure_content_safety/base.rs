use std::time::Duration;

use betterlite_http::proxy::limits::{ReadError, read_bounded};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::api_version::resolve_api_version;
use super::error::AzureContentSafetyError;

// The fallback httpx applies when a guardrail config sets no timeout of its own.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const SUBSCRIPTION_KEY: &str = "ocp-apim-subscription-key";

#[derive(Debug, Clone)]
pub struct AzureGuardrailBase {
    pub async_handler: reqwest::Client,
    pub api_key: SecretString,
    pub api_base: String,
    pub api_version: Option<String>,
    pub timeout: Option<Duration>,
}

impl AzureGuardrailBase {
    pub async fn post_to_content_safety<Body, Reply>(
        &self,
        endpoint_path: &str,
        request_body: &Body,
    ) -> Result<Reply, AzureContentSafetyError>
    where
        Body: Serialize + Sync,
        Reply: DeserializeOwned,
    {
        let api_version = resolve_api_version(self.api_version.as_deref());
        let url = format!(
            "{}/contentsafety/{endpoint_path}?api-version={api_version}",
            self.api_base
        );
        let mut headers = HeaderMap::new();
        let mut subscription_key = HeaderValue::from_str(self.api_key.expose_secret())
            .map_err(|source| AzureContentSafetyError::InvalidApiKey { source })?;
        subscription_key.set_sensitive(true);
        headers.insert(HeaderName::from_static(SUBSCRIPTION_KEY), subscription_key);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let mut response = self
            .async_handler
            .post(url)
            .headers(headers)
            .json(request_body)
            .timeout(self.timeout.unwrap_or(DEFAULT_TIMEOUT))
            .send()
            .await
            .map_err(request_error)?;
        let body = read_bounded(&mut response, MAX_RESPONSE_BYTES)
            .await
            .map_err(read_error)?;
        serde_json::from_slice(&body)
            .map_err(|source| AzureContentSafetyError::ResponseJson { source })
    }
}

fn request_error(source: reqwest::Error) -> AzureContentSafetyError {
    if source.is_timeout() {
        AzureContentSafetyError::Timeout { source }
    } else {
        AzureContentSafetyError::Request { source }
    }
}

fn read_error(error: ReadError) -> AzureContentSafetyError {
    match error {
        ReadError::TooLarge { limit } => AzureContentSafetyError::ResponseTooLarge { limit },
        ReadError::Failed(source) => AzureContentSafetyError::Request { source },
    }
}
