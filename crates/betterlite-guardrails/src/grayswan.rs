use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailInputType, GuardrailMessage,
};

mod format;
pub mod wire;

use format::{
    category_override, conversation_messages, format_violation_message, reasoning_mode_override,
    text_override, violation_guardrail_message,
};
use wire::{GraySwanInput, GraySwanPayload, GraySwanResponse, GraySwanResult};

pub const DEFAULT_API_BASE: &str = "https://api.grayswan.ai";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnFlaggedAction {
    #[serde(rename = "BLOCK")]
    Block,
    #[serde(rename = "MONITOR")]
    Monitor,
    #[serde(rename = "PASSTHROUGH")]
    Passthrough,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningMode {
    Off,
    Hybrid,
    Thinking,
}

#[derive(Debug, Clone)]
pub struct GraySwanSettings {
    pub on_flagged_action: OnFlaggedAction,
    pub violation_threshold: f64,
    pub reasoning_mode: Option<ReasoningMode>,
    pub categories: Option<BTreeMap<String, String>>,
    pub policy_id: Option<String>,
    pub fail_open: bool,
}

impl Default for GraySwanSettings {
    fn default() -> Self {
        Self {
            on_flagged_action: OnFlaggedAction::Monitor,
            violation_threshold: 0.5,
            reasoning_mode: None,
            categories: None,
            policy_id: None,
            fail_open: true,
        }
    }
}

#[derive(Debug, Error)]
pub enum GraySwanGuardrailAPIError {
    #[error("the Gray Swan API request timed out")]
    Timeout { source: reqwest::Error },
    #[error("the Gray Swan API could not be reached")]
    Request { source: reqwest::Error },
    #[error("the Gray Swan API returned status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Gray Swan API key is not a valid header")]
    InvalidHeader { source: InvalidHeaderValue },
    #[error("the Gray Swan API response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Gray Swan API response is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("Gray Swan flagged content{reason}")]
    Blocked { reason: String },
    #[error("Gray Swan passthrough requested before the model call{reason}")]
    PreCallPassthrough { reason: String },
}

impl GraySwanGuardrailAPIError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Timeout { .. }
            | Self::Request { .. }
            | Self::HttpStatus { .. }
            | Self::InvalidHeader { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::Blocked { .. }
            | Self::PreCallPassthrough { .. } => StatusCode::BAD_REQUEST,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GraySwanGuardrail {
    pub api_key: SecretString,
    pub api_base: String,
    pub settings: GraySwanSettings,
    pub client: reqwest::Client,
    pub timeout: Duration,
}

impl GraySwanGuardrail {
    #[must_use]
    pub fn with_client(
        api_key: SecretString,
        api_base: Option<String>,
        settings: GraySwanSettings,
        client: reqwest::Client,
        timeout: Duration,
    ) -> Self {
        let resolved_base = api_base.unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        Self {
            api_key,
            api_base: resolved_base,
            settings,
            client,
            timeout,
        }
    }

    pub fn prepare_headers(&self) -> Result<HeaderMap, GraySwanGuardrailAPIError> {
        let mut headers = HeaderMap::new();
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {}", self.api_key.expose_secret()))
                .map_err(|source| GraySwanGuardrailAPIError::InvalidHeader { source })?;
        authorization.set_sensitive(true);
        headers.insert(AUTHORIZATION, authorization);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(headers)
    }

    #[must_use]
    pub fn prepare_payload(&self, input: &GraySwanInput) -> GraySwanPayload {
        let categories = category_override(&input.metadata)
            .or_else(|| self.settings.categories.clone())
            .filter(|values| !values.is_empty());
        let policy_id =
            text_override(&input.metadata, "policy_id").or_else(|| self.settings.policy_id.clone());
        let reasoning_mode =
            reasoning_mode_override(&input.metadata).or(self.settings.reasoning_mode);
        let metadata = input.metadata.get("metadata").cloned();
        let litellm_metadata = if input.litellm_metadata.is_empty() {
            None
        } else {
            Some(GuardrailExtraValue::Object(input.litellm_metadata.clone()))
        };
        GraySwanPayload {
            messages: conversation_messages(input),
            tools: input.tools.clone(),
            categories,
            policy_id,
            reasoning_mode,
            metadata,
            litellm_metadata,
        }
    }

    pub async fn apply_guardrail(
        &self,
        input: GraySwanInput,
        input_type: GuardrailInputType,
    ) -> Result<GraySwanResult, GraySwanGuardrailAPIError> {
        let original = conversation_messages(&input);
        if input.messages.is_empty() {
            return Ok(GraySwanResult {
                flagged: false,
                messages: original,
                violation_message: None,
            });
        }
        match self.call_api(&self.prepare_payload(&input)).await {
            Ok(response) => self.process_response(&response, input_type, original),
            Err(error) if self.settings.fail_open => {
                tracing::error!(error = %error, "Gray Swan API error; passing through");
                Ok(GraySwanResult {
                    flagged: false,
                    messages: original,
                    violation_message: None,
                })
            }
            Err(error) => Err(error),
        }
    }

    pub async fn call_api(
        &self,
        payload: &GraySwanPayload,
    ) -> Result<GraySwanResponse, GraySwanGuardrailAPIError> {
        let endpoint = format!("{}/cygnal/monitor", self.api_base.trim_end_matches('/'));
        let response = self
            .client
            .post(endpoint)
            .headers(self.prepare_headers()?)
            .json(payload)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(GraySwanGuardrailAPIError::HttpStatus { status });
        }
        let body = read_capped_body(response, MAX_RESPONSE_BYTES).await?;
        serde_json::from_slice(&body)
            .map_err(|source| GraySwanGuardrailAPIError::ResponseJson { source })
    }

    pub fn process_response(
        &self,
        response: &GraySwanResponse,
        input_type: GuardrailInputType,
        original: Vec<GuardrailMessage>,
    ) -> Result<GraySwanResult, GraySwanGuardrailAPIError> {
        let flagged = response.violation >= self.settings.violation_threshold;
        if !flagged || self.settings.on_flagged_action == OnFlaggedAction::Monitor {
            return Ok(GraySwanResult {
                flagged,
                messages: original,
                violation_message: None,
            });
        }
        let is_output = input_type == GuardrailInputType::Response;
        let violation_message = format_violation_message(response, is_output);
        let reason = format!(": {violation_message}");
        match (self.settings.on_flagged_action, input_type) {
            (OnFlaggedAction::Block, _) => Err(GraySwanGuardrailAPIError::Blocked { reason }),
            (OnFlaggedAction::Passthrough, GuardrailInputType::Request) => {
                Err(GraySwanGuardrailAPIError::PreCallPassthrough { reason })
            }
            (OnFlaggedAction::Passthrough, GuardrailInputType::Response) => Ok(GraySwanResult {
                flagged: true,
                messages: vec![violation_guardrail_message(&violation_message)],
                violation_message: Some(violation_message),
            }),
            (OnFlaggedAction::Monitor, _) => Ok(GraySwanResult {
                flagged,
                messages: original,
                violation_message: Some(violation_message),
            }),
        }
    }
}

fn request_error(source: reqwest::Error) -> GraySwanGuardrailAPIError {
    if source.is_timeout() {
        GraySwanGuardrailAPIError::Timeout { source }
    } else {
        GraySwanGuardrailAPIError::Request { source }
    }
}

async fn read_capped_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, GraySwanGuardrailAPIError> {
    let limit_bytes = u64::try_from(limit).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > limit_bytes)
    {
        return Err(GraySwanGuardrailAPIError::ResponseTooLarge { limit });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Err(GraySwanGuardrailAPIError::ResponseTooLarge { limit });
        }
    }
    Ok(body)
}

#[path = "grayswan/tests.rs"]
mod tests;
