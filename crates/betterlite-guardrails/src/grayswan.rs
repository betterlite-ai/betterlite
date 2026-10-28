use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailInputType, GuardrailMessage, GuardrailToolParam,
};

pub const DEFAULT_API_BASE: &str = "https://api.grayswan.ai";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnFlaggedAction {
    #[serde(rename = "BLOCK")]
    Block,
    #[serde(rename = "MONITOR")]
    Monitor,
    #[serde(rename = "PASSTHROUGH")]
    Passthrough,
}

#[derive(Debug, Clone)]
pub struct GraySwanSettings {
    pub on_flagged_action: OnFlaggedAction,
    pub violation_threshold: f64,
    pub reasoning_mode: Option<String>,
    pub categories: Option<Vec<String>>,
    pub policy_id: Option<String>,
    pub fail_open: bool,
}

impl Default for GraySwanSettings {
    fn default() -> Self {
        Self {
            on_flagged_action: OnFlaggedAction::Block,
            violation_threshold: 0.5,
            reasoning_mode: None,
            categories: None,
            policy_id: None,
            fail_open: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraySwanInput {
    pub messages: Vec<GuardrailMessage>,
    #[serde(default)]
    pub tools: Vec<GuardrailToolParam>,
    #[serde(default)]
    pub metadata: BTreeMap<String, GuardrailExtraValue>,
    #[serde(default)]
    pub request_history: Vec<GuardrailMessage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraySwanPayload {
    pub messages: Vec<GuardrailMessage>,
    pub tools: Vec<GuardrailToolParam>,
    pub parameters: BTreeMap<String, GuardrailExtraValue>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraySwanResponse {
    #[serde(default)]
    pub violation_score: f64,
    #[serde(default)]
    pub violation_reason: Option<String>,
    #[serde(default)]
    pub messages: Option<Vec<GuardrailMessage>>,
}

#[derive(Debug, Clone)]
pub struct GraySwanResult {
    pub flagged: bool,
    pub messages: Vec<GuardrailMessage>,
    pub violation_reason: Option<String>,
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
    #[error("the Gray Swan API response is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("Gray Swan flagged content{reason}")]
    Blocked { reason: String },
    #[error("Gray Swan passthrough requested before the model call{reason}")]
    PreCallPassthrough { reason: String },
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
    pub fn new(
        api_key: SecretString,
        api_base: Option<String>,
        settings: GraySwanSettings,
    ) -> Self {
        let resolved_base = api_base.unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
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
        let mut parameters = BTreeMap::new();
        insert_parameter(
            &mut parameters,
            "categories",
            self.settings.categories.clone().map(|values| {
                GuardrailExtraValue::Array(
                    values.into_iter().map(GuardrailExtraValue::Text).collect(),
                )
            }),
            &input.metadata,
        );
        insert_parameter(
            &mut parameters,
            "policy_id",
            self.settings
                .policy_id
                .clone()
                .map(GuardrailExtraValue::Text),
            &input.metadata,
        );
        let reasoning = input.metadata.get("reasoning_mode").cloned().or_else(|| {
            self.settings
                .reasoning_mode
                .clone()
                .map(GuardrailExtraValue::Text)
        });
        if let Some(value) = reasoning {
            parameters.insert("reasoning_mode".to_owned(), value);
        }
        GraySwanPayload {
            messages: conversation_messages(input),
            tools: input.tools.clone(),
            parameters,
        }
    }

    pub async fn apply_guardrail(
        &self,
        input: GraySwanInput,
        input_type: GuardrailInputType,
    ) -> Result<GraySwanResult, GraySwanGuardrailAPIError> {
        let original = conversation_messages(&input);
        match self.call_api(&self.prepare_payload(&input)).await {
            Ok(response) => self.process_response(response, input_type, original),
            Err(error) if self.settings.fail_open => {
                tracing::error!(error = %error, "Gray Swan API error; passing through");
                Ok(GraySwanResult {
                    flagged: false,
                    messages: original,
                    violation_reason: None,
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
        let body = response.bytes().await.map_err(request_error)?;
        serde_json::from_slice(&body)
            .map_err(|source| GraySwanGuardrailAPIError::ResponseJson { source })
    }

    pub fn process_response(
        &self,
        response: GraySwanResponse,
        input_type: GuardrailInputType,
        original: Vec<GuardrailMessage>,
    ) -> Result<GraySwanResult, GraySwanGuardrailAPIError> {
        let flagged = response.violation_score >= self.settings.violation_threshold;
        if !flagged || self.settings.on_flagged_action == OnFlaggedAction::Monitor {
            return Ok(GraySwanResult {
                flagged,
                messages: original,
                violation_reason: response.violation_reason,
            });
        }
        let reason = response
            .violation_reason
            .as_deref()
            .map_or_else(String::new, |value| format!(": {value}"));
        match (self.settings.on_flagged_action, input_type) {
            (OnFlaggedAction::Block, _) => Err(GraySwanGuardrailAPIError::Blocked { reason }),
            (OnFlaggedAction::Passthrough, GuardrailInputType::Request) => {
                Err(GraySwanGuardrailAPIError::PreCallPassthrough { reason })
            }
            (OnFlaggedAction::Passthrough, GuardrailInputType::Response) => Ok(GraySwanResult {
                flagged: true,
                messages: response.messages.unwrap_or(original),
                violation_reason: response.violation_reason,
            }),
            (OnFlaggedAction::Monitor, _) => Ok(GraySwanResult {
                flagged,
                messages: original,
                violation_reason: response.violation_reason,
            }),
        }
    }
}

fn insert_parameter(
    parameters: &mut BTreeMap<String, GuardrailExtraValue>,
    name: &str,
    configured: Option<GuardrailExtraValue>,
    metadata: &BTreeMap<String, GuardrailExtraValue>,
) {
    match (metadata.get(name), configured) {
        (Some(value), _) => {
            parameters.insert(name.to_owned(), value.clone());
        }
        (None, Some(value)) => {
            parameters.insert(name.to_owned(), value);
        }
        (None, None) => {}
    }
}

fn conversation_messages(input: &GraySwanInput) -> Vec<GuardrailMessage> {
    let mut messages = input.request_history.clone();
    messages.extend(input.messages.clone());
    messages
}

fn request_error(source: reqwest::Error) -> GraySwanGuardrailAPIError {
    if source.is_timeout() {
        GraySwanGuardrailAPIError::Timeout { source }
    } else {
        GraySwanGuardrailAPIError::Request { source }
    }
}

#[path = "grayswan/tests.rs"]
mod tests;
