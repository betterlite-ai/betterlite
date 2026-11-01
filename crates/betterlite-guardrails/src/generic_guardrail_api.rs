use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::generic_guardrail_contracts::{
    GenericGuardrailAPIMetadata, GenericGuardrailAPIRequest, GenericGuardrailAPIResponse,
    GenericGuardrailAPISettings, GuardrailExtraValue, UnreachableFallback,
};
mod response;
use response::{enforce_response, passthrough_response};
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum GenericGuardrailAPIError {
    #[error("the generic guardrail API request timed out")]
    Timeout { source: reqwest::Error },
    #[error("the generic guardrail API could not be reached")]
    Request { source: reqwest::Error },
    #[error("the generic guardrail API key is not a valid header")]
    InvalidHeader { source: InvalidHeaderValue },
    #[error("the generic guardrail API returned status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the generic guardrail API response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the generic guardrail API response is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("the generic guardrail API blocked the content{reason}")]
    Blocked { reason: String },
}
impl GenericGuardrailAPIError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Blocked { .. }
            | Self::Timeout { .. }
            | Self::Request { .. }
            | Self::InvalidHeader { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => StatusCode::BAD_REQUEST,
        }
    }
    const fn is_unreachable(&self) -> bool {
        match self {
            Self::Timeout { .. } | Self::Request { .. } => true,
            Self::HttpStatus { status } => matches!(
                *status,
                StatusCode::BAD_GATEWAY
                    | StatusCode::SERVICE_UNAVAILABLE
                    | StatusCode::GATEWAY_TIMEOUT
            ),
            Self::Blocked { .. }
            | Self::InvalidHeader { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => false,
        }
    }
}
#[derive(Debug, Clone)]
pub struct GenericGuardrailAPI {
    pub api_key: Option<SecretString>,
    pub api_base: String,
    pub settings: GenericGuardrailAPISettings,
    pub provider_params: BTreeMap<String, GuardrailExtraValue>,
    pub client: reqwest::Client,
    pub timeout: Duration,
}
impl GenericGuardrailAPI {
    pub fn prepare_headers(&self) -> Result<HeaderMap, GenericGuardrailAPIError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(api_key) = &self.api_key {
            let mut value = HeaderValue::from_str(api_key.expose_secret())
                .map_err(|source| GenericGuardrailAPIError::InvalidHeader { source })?;
            value.set_sensitive(true);
            headers.insert(HeaderName::from_static("x-api-key"), value);
        }
        Ok(headers)
    }
    fn merge_provider_params(
        &self,
        request: GenericGuardrailAPIRequest,
    ) -> GenericGuardrailAPIRequest {
        let mut merged = self.provider_params.clone();
        if let Some(dynamic_params) = &request.provider_params {
            merged.extend(dynamic_params.clone());
        }
        GenericGuardrailAPIRequest {
            provider_params: Some(merged),
            ..request
        }
    }
    pub async fn apply_guardrail(
        &self,
        request: GenericGuardrailAPIRequest,
    ) -> Result<GenericGuardrailAPIResponse, GenericGuardrailAPIError> {
        let merged = self.merge_provider_params(request.clone());
        let fallback = passthrough_response(&request);
        match self.call_api(&merged).await {
            Ok(response) => enforce_response(response, &request, &merged),
            Err(error) => self.handle_error(error, fallback),
        }
    }
    pub async fn call_api(
        &self,
        request: &GenericGuardrailAPIRequest,
    ) -> Result<GenericGuardrailAPIResponse, GenericGuardrailAPIError> {
        let response = self
            .client
            .post(&self.api_base)
            .headers(self.prepare_headers()?)
            .json(request)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(GenericGuardrailAPIError::HttpStatus { status });
        }
        let body = read_capped_body(response, MAX_RESPONSE_BYTES).await?;
        serde_json::from_slice(&body)
            .map_err(|source| GenericGuardrailAPIError::ResponseJson { source })
    }
    fn handle_error(
        &self,
        error: GenericGuardrailAPIError,
        fallback: GenericGuardrailAPIResponse,
    ) -> Result<GenericGuardrailAPIResponse, GenericGuardrailAPIError> {
        let unreachable_fail_open = error.is_unreachable()
            && self.settings.unreachable_fallback == UnreachableFallback::FailOpen;
        if unreachable_fail_open || !self.settings.fail_on_error {
            tracing::error!(error = %error, "generic guardrail API error; passing through");
            return Ok(fallback);
        }
        Err(error)
    }
}
#[must_use]
pub fn extract_user_api_key_metadata(
    litellm_metadata: &BTreeMap<String, Option<String>>,
    metadata: &BTreeMap<String, Option<String>>,
) -> GenericGuardrailAPIMetadata {
    let mut merged = metadata.clone();
    merged.extend(litellm_metadata.clone());
    let mut result = GenericGuardrailAPIMetadata {
        user_api_key_hash: merged.get("user_api_key_hash").cloned().flatten(),
        user_api_key_alias: merged.get("user_api_key_alias").cloned().flatten(),
        user_api_key_user_id: merged.get("user_api_key_user_id").cloned().flatten(),
        user_api_key_user_email: merged.get("user_api_key_user_email").cloned().flatten(),
        user_api_key_team_id: merged.get("user_api_key_team_id").cloned().flatten(),
        user_api_key_team_alias: merged.get("user_api_key_team_alias").cloned().flatten(),
        user_api_key_end_user_id: merged.get("user_api_key_end_user_id").cloned().flatten(),
        user_api_key_org_id: merged.get("user_api_key_org_id").cloned().flatten(),
        extra: BTreeMap::new(),
    };
    if let Some(token) = merged.get("user_api_key_token").cloned().flatten() {
        result.user_api_key_hash = Some(token);
    }
    result
}
fn request_error(source: reqwest::Error) -> GenericGuardrailAPIError {
    if source.is_timeout() {
        GenericGuardrailAPIError::Timeout { source }
    } else {
        GenericGuardrailAPIError::Request { source }
    }
}
async fn read_capped_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, GenericGuardrailAPIError> {
    let limit_bytes = u64::try_from(limit).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > limit_bytes)
    {
        return Err(GenericGuardrailAPIError::ResponseTooLarge { limit });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Err(GenericGuardrailAPIError::ResponseTooLarge { limit });
        }
    }
    Ok(body)
}
#[path = "generic_guardrail_api/tests.rs"]
mod tests;
