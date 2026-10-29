use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailInputType, GuardrailMessage,
};

pub mod error;
mod format;
pub mod wire;

use error::GraySwanGuardrailAPIError;
use format::{build_messages, format_violation_message};
use wire::{GraySwanInput, GraySwanPayload, GraySwanResponse, GraySwanResult};

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

#[derive(Debug, Clone)]
pub struct GraySwanGuardrail {
    pub api_key: SecretString,
    pub api_base: String,
    pub settings: GraySwanSettings,
    pub client: reqwest::Client,
    pub timeout: Duration,
}

impl GraySwanGuardrail {
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
    pub fn prepare_payload(
        &self,
        input: &GraySwanInput,
        messages: Vec<GuardrailMessage>,
    ) -> GraySwanPayload {
        let categories = input
            .metadata
            .categories
            .clone()
            .or_else(|| self.settings.categories.clone())
            .filter(|values| !values.is_empty());
        let policy_id = input
            .metadata
            .policy_id
            .clone()
            .or_else(|| self.settings.policy_id.clone());
        let reasoning_mode = input
            .metadata
            .reasoning_mode
            .or(self.settings.reasoning_mode);
        let metadata = input.metadata.metadata.clone();
        let litellm_metadata = if input.litellm_metadata.is_empty() {
            None
        } else {
            Some(GuardrailExtraValue::Object(input.litellm_metadata.clone()))
        };
        GraySwanPayload {
            messages,
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
        let tool_calls: &[GuardrailExtraValue] = if input_type == GuardrailInputType::Response {
            &input.tool_calls
        } else {
            &[]
        };
        let context = if input_type == GuardrailInputType::Response {
            input.request_history.clone()
        } else {
            Vec::new()
        };
        if input.texts.is_empty() && tool_calls.is_empty() {
            return Ok(GraySwanResult {
                flagged: false,
                messages: context,
                violation_message: None,
            });
        }
        let messages = build_messages(&input.texts, tool_calls, input_type, context);
        let payload = self.prepare_payload(&input, messages.clone());
        match self.call_api(&payload).await {
            Ok(response) => self.process_response(&response, input_type, messages),
            Err(error) if self.settings.fail_open => {
                tracing::error!(error = %error, "Gray Swan API error; passing through");
                Ok(GraySwanResult {
                    flagged: false,
                    messages,
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
        if !flagged {
            return Ok(GraySwanResult {
                flagged: false,
                messages: original,
                violation_message: None,
            });
        }
        let is_output = input_type == GuardrailInputType::Response;
        let violation_message = format_violation_message(response, is_output);
        match self.settings.on_flagged_action {
            OnFlaggedAction::Monitor => Ok(GraySwanResult {
                flagged,
                messages: original,
                violation_message: Some(violation_message),
            }),
            OnFlaggedAction::Block => Err(GraySwanGuardrailAPIError::Blocked {
                reason: format!(": {violation_message}"),
            }),
            OnFlaggedAction::Passthrough => {
                let guardrail_message = GuardrailMessage {
                    role: "assistant".to_owned(),
                    extra: BTreeMap::from([(
                        "content".to_owned(),
                        GuardrailExtraValue::Text(violation_message.clone()),
                    )]),
                };
                Ok(GraySwanResult {
                    flagged: true,
                    messages: vec![guardrail_message],
                    violation_message: Some(violation_message),
                })
            }
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
