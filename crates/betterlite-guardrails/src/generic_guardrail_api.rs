use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::generic_guardrail_contracts::{
    GenericGuardrailAPIMetadata, GenericGuardrailAPIRequest, GenericGuardrailAPIResponse,
    GenericGuardrailAPISettings, GuardrailResponseAction, UnreachableFallback,
    reconcile_structured_rows,
};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_PATH: &str = "/beta/litellm_basic_guardrail_api";
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
            Self::Timeout { .. } | Self::Request { .. } | Self::HttpStatus { .. } => true,
            Self::Blocked { .. }
            | Self::InvalidHeader { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GenericGuardrailAPI {
    pub api_key: SecretString,
    pub api_base: String,
    pub settings: GenericGuardrailAPISettings,
    pub client: reqwest::Client,
    pub timeout: Duration,
}

impl GenericGuardrailAPI {
    #[must_use]
    pub fn new(
        api_key: SecretString,
        api_base: &str,
        settings: GenericGuardrailAPISettings,
    ) -> Self {
        let resolved_base = normalize_api_base(api_base);
        Self {
            api_key,
            api_base: resolved_base,
            settings,
            client: reqwest::Client::default(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub fn with_client(
        api_key: SecretString,
        api_base: &str,
        settings: GenericGuardrailAPISettings,
        client: reqwest::Client,
        timeout: Duration,
    ) -> Self {
        let resolved_base = normalize_api_base(api_base);
        Self {
            api_key,
            api_base: resolved_base,
            settings,
            client,
            timeout,
        }
    }

    pub fn prepare_headers(&self) -> Result<HeaderMap, GenericGuardrailAPIError> {
        let mut headers = HeaderMap::new();
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {}", self.api_key.expose_secret()))
                .map_err(|source| GenericGuardrailAPIError::InvalidHeader { source })?;
        authorization.set_sensitive(true);
        headers.insert(AUTHORIZATION, authorization);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(headers)
    }

    pub async fn apply_guardrail(
        &self,
        request: GenericGuardrailAPIRequest,
    ) -> Result<GenericGuardrailAPIResponse, GenericGuardrailAPIError> {
        let shown = request.clone();
        self.apply_guardrail_with_original(shown, &request).await
    }

    pub async fn apply_guardrail_with_original(
        &self,
        request: GenericGuardrailAPIRequest,
        original: &GenericGuardrailAPIRequest,
    ) -> Result<GenericGuardrailAPIResponse, GenericGuardrailAPIError> {
        let fallback = passthrough_response(original);
        match self.call_api(&request).await {
            Ok(mut response) => {
                reconcile_response(&mut response, original, &request);
                enforce_response(response)
            }
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
        let body = response.bytes().await.map_err(request_error)?;
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(GenericGuardrailAPIError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        serde_json::from_slice(&body)
            .map_err(|source| GenericGuardrailAPIError::ResponseJson { source })
    }

    fn handle_error(
        &self,
        error: GenericGuardrailAPIError,
        fallback: GenericGuardrailAPIResponse,
    ) -> Result<GenericGuardrailAPIResponse, GenericGuardrailAPIError> {
        if self.settings.fail_on_error {
            tracing::error!(error = %error, "generic guardrail API error; passing through");
            return Ok(fallback);
        }
        if error.is_unreachable()
            && self.settings.unreachable_fallback == UnreachableFallback::FailOpen
        {
            tracing::error!(error = %error, "generic guardrail API unreachable; passing through");
            return Ok(fallback);
        }
        Err(error)
    }
}

#[must_use]
pub fn extract_user_api_key_metadata(
    litellm_metadata: Option<&BTreeMap<String, Option<String>>>,
    metadata: Option<&BTreeMap<String, Option<String>>>,
) -> GenericGuardrailAPIMetadata {
    let mut result = GenericGuardrailAPIMetadata::default();
    if let Some(values) = metadata {
        merge_metadata(&mut result, values);
    }
    if let Some(values) = litellm_metadata {
        merge_metadata(&mut result, values);
    }
    if let Some(token) = result.extra.remove("user_api_key_token").flatten() {
        result.user_api_key_hash = Some(token);
    }
    result
}

fn merge_metadata(
    target: &mut GenericGuardrailAPIMetadata,
    values: &BTreeMap<String, Option<String>>,
) {
    for (name, value) in values {
        if !name.starts_with("user_api_key_") {
            continue;
        }
        match name.as_str() {
            "user_api_key_hash" => target.user_api_key_hash.clone_from(value),
            "user_api_key_alias" => target.user_api_key_alias.clone_from(value),
            "user_api_key_user_id" => target.user_api_key_user_id.clone_from(value),
            "user_api_key_user_email" => target.user_api_key_user_email.clone_from(value),
            "user_api_key_team_id" => target.user_api_key_team_id.clone_from(value),
            "user_api_key_team_alias" => target.user_api_key_team_alias.clone_from(value),
            "user_api_key_end_user_id" => target.user_api_key_end_user_id.clone_from(value),
            "user_api_key_org_id" => target.user_api_key_org_id.clone_from(value),
            _ => {
                target.extra.insert(name.clone(), value.clone());
            }
        }
    }
}

fn normalize_api_base(api_base: &str) -> String {
    let trimmed = api_base.trim_end_matches('/');
    if trimmed.ends_with(DEFAULT_PATH) {
        trimmed.to_owned()
    } else {
        format!("{trimmed}{DEFAULT_PATH}")
    }
}

fn request_error(source: reqwest::Error) -> GenericGuardrailAPIError {
    if source.is_timeout() {
        GenericGuardrailAPIError::Timeout { source }
    } else {
        GenericGuardrailAPIError::Request { source }
    }
}

fn passthrough_response(request: &GenericGuardrailAPIRequest) -> GenericGuardrailAPIResponse {
    let texts = request.texts.clone();
    let images = request.images.clone();
    let tools = request.tools.clone();
    let structured_messages = request.structured_messages.clone();
    GenericGuardrailAPIResponse {
        action: GuardrailResponseAction::None,
        texts,
        images,
        tools,
        structured_messages,
        blocked_reason: None,
        stream_holdback_chars: None,
    }
}

fn reconcile_response(
    response: &mut GenericGuardrailAPIResponse,
    original: &GenericGuardrailAPIRequest,
    shown: &GenericGuardrailAPIRequest,
) {
    if let (Some(original_rows), Some(shown_rows), Some(returned_rows)) = (
        original.structured_messages.as_deref(),
        shown.structured_messages.as_deref(),
        response.structured_messages.as_deref(),
    ) {
        response.structured_messages =
            reconcile_structured_rows(original_rows, shown_rows, returned_rows);
    }
}

fn enforce_response(
    response: GenericGuardrailAPIResponse,
) -> Result<GenericGuardrailAPIResponse, GenericGuardrailAPIError> {
    if response.action == GuardrailResponseAction::Blocked {
        let reason = response
            .blocked_reason
            .as_deref()
            .map_or_else(String::new, |value| format!(": {value}"));
        return Err(GenericGuardrailAPIError::Blocked { reason });
    }
    Ok(response)
}

#[path = "generic_guardrail_api/tests.rs"]
mod tests;
