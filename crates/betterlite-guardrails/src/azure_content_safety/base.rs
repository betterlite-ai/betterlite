use std::time::Duration;

use betterlite_http::proxy::limits::read_bounded;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::{AzureContentSafetyError, read_error, request_error};

// The fallback httpx applies when a guardrail config sets no timeout of its own.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
pub const SUBSCRIPTION_KEY: &str = "ocp-apim-subscription-key";
// AsyncHTTPHandler turns httpx's timeout into litellm.Timeout, whose status the proxy answers with.
const TIMEOUT_STATUS: StatusCode = StatusCode::REQUEST_TIMEOUT;

#[derive(Debug, Clone)]
pub struct AzureGuardrailBase {
    pub async_handler: reqwest::Client,
    pub api_key: SecretString,
    pub api_base: String,
    pub api_version: String,
    pub timeout: Duration,
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
        let url = format!(
            "{}/contentsafety/{endpoint_path}?api-version={}",
            self.api_base, self.api_version
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
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|source| request_error(source, TIMEOUT_STATUS))?;
        let status = response.status();
        let body = read_bounded(&mut response, MAX_RESPONSE_BYTES)
            .await
            .map_err(|error| read_error(error, TIMEOUT_STATUS))?;
        if !status.is_success() {
            return Err(AzureContentSafetyError::HttpStatus {
                status,
                body: String::from_utf8(body).unwrap_or_default(),
            });
        }
        serde_json::from_slice(&body)
            .map_err(|source| AzureContentSafetyError::ResponseJson { source })
    }
}
