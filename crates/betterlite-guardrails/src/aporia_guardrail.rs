use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use crate::generic_guardrail_contracts::GuardrailMessage;

pub mod error;
use error::AporiaGuardrailError;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const SUPPORTED_OPENAI_ROLES: [&str; 3] = ["system", "user", "assistant"];

pub const SUPPORTED_EVENT_HOOKS: [EventHook; 2] = [EventHook::DuringCall, EventHook::PostCall];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EventHook {
    #[serde(rename = "during_call")]
    DuringCall,
    #[serde(rename = "post_call")]
    PostCall,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub api_key: SecretString,
    pub api_base: String,
}

impl Settings {
    // See docs/deviations/B3753.yaml: LiteLLM raises an uncaught KeyError here instead.
    pub fn resolve<L>(
        api_key: Option<String>,
        api_base: Option<String>,
        lookup: &L,
    ) -> Result<Self, AporiaGuardrailError>
    where
        L: EnvironmentLookup,
    {
        let resolved_api_key = api_key.or_else(|| lookup.lookup("APORIO_API_KEY")).ok_or(
            AporiaGuardrailError::MissingEnvironment {
                name: "APORIO_API_KEY",
            },
        )?;
        let resolved_api_base = api_base
            .or_else(|| lookup.lookup("APORIO_API_BASE"))
            .ok_or(AporiaGuardrailError::MissingEnvironment {
                name: "APORIO_API_BASE",
            })?;
        Ok(Self {
            api_key: SecretString::from(resolved_api_key),
            api_base: resolved_api_base,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationTarget {
    Prompt,
    Response,
    Both,
}

#[derive(Debug, Clone, Serialize)]
pub struct AporiaApiRequest {
    pub messages: Vec<GuardrailMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_target: Option<ValidationTarget>,
    #[serde(flatten)]
    pub dynamic_params: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AporiaApiResponse {
    pub action: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, GuardrailValue>,
}

// Mirrors LiteLLM's "if new_messages and response_string:" truthiness check.
#[must_use]
pub fn prepare_aporia_request(
    messages: Vec<GuardrailMessage>,
    response_string: Option<String>,
    dynamic_params: BTreeMap<String, GuardrailValue>,
) -> AporiaApiRequest {
    let has_messages = !messages.is_empty();
    let has_response = response_string
        .as_deref()
        .is_some_and(|value| !value.is_empty());
    let validation_target = match (has_messages, has_response) {
        (true, true) => Some(ValidationTarget::Both),
        (true, false) => Some(ValidationTarget::Prompt),
        (false, true) => Some(ValidationTarget::Response),
        (false, false) => None,
    };
    AporiaApiRequest {
        messages,
        response: response_string,
        validation_target,
        dynamic_params,
    }
}

#[derive(Debug, Clone)]
pub struct AporiaGuardrail {
    pub settings: Settings,
    pub client: reqwest::Client,
    pub timeout: Duration,
}

impl AporiaGuardrail {
    #[must_use]
    pub fn transform_messages(messages: Vec<GuardrailMessage>) -> Vec<GuardrailMessage> {
        messages
            .into_iter()
            .map(|message| {
                if SUPPORTED_OPENAI_ROLES.contains(&message.role.as_str()) {
                    message
                } else {
                    GuardrailMessage {
                        role: "other".to_owned(),
                        extra: message.extra,
                    }
                }
            })
            .collect()
    }

    pub async fn make_aporia_api_request(
        &self,
        messages: Vec<GuardrailMessage>,
        response_string: Option<String>,
        dynamic_params: &BTreeMap<String, GuardrailValue>,
    ) -> Result<(), AporiaGuardrailError> {
        let prepared = prepare_aporia_request(messages, response_string, dynamic_params.clone());
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let mut api_key = HeaderValue::from_str(self.settings.api_key.expose_secret())
            .map_err(|source| AporiaGuardrailError::InvalidHeader { source })?;
        api_key.set_sensitive(true);
        headers.insert(HeaderName::from_static("x-aporia-api-key"), api_key);
        let response = self
            .client
            .post(format!(
                "{}/validate",
                self.settings.api_base.trim_end_matches('/')
            ))
            .headers(headers)
            .json(&prepared)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(AporiaGuardrailError::HttpStatus { status });
        }
        let bytes = read_capped_body(response, MAX_RESPONSE_BYTES).await?;
        let parsed: AporiaApiResponse = serde_json::from_slice(&bytes)
            .map_err(|source| AporiaGuardrailError::ResponseJson { source })?;
        if parsed.action.as_deref() == Some("block") {
            return Err(AporiaGuardrailError::Blocked { response: parsed });
        }
        Ok(())
    }

    // Returns whether the guardrail ran, for the (out-of-scope) applied-guardrails header.
    pub async fn async_post_call_success_hook(
        &self,
        should_run: bool,
        response_string: Option<String>,
        messages: Vec<GuardrailMessage>,
        dynamic_params: &BTreeMap<String, GuardrailValue>,
    ) -> Result<bool, AporiaGuardrailError> {
        if !should_run {
            return Ok(false);
        }
        let Some(response_text) = response_string else {
            return Ok(false);
        };
        self.make_aporia_api_request(messages, Some(response_text), dynamic_params)
            .await?;
        Ok(true)
    }

    // Returns whether the guardrail ran, for the (out-of-scope) applied-guardrails header.
    pub async fn async_moderation_hook(
        &self,
        gate: ModerationGate,
        messages: Option<Vec<GuardrailMessage>>,
        dynamic_params: &BTreeMap<String, GuardrailValue>,
    ) -> Result<bool, AporiaGuardrailError> {
        if !gate.should_run || !gate.should_proceed {
            return Ok(false);
        }
        let Some(incoming_messages) = messages else {
            tracing::warn!("Aporia AI: not running guardrail. No messages in data");
            return Ok(false);
        };
        let transformed = Self::transform_messages(incoming_messages);
        self.make_aporia_api_request(transformed, None, dynamic_params)
            .await?;
        Ok(true)
    }
}

// Bundles the two skip conditions LiteLLM checks since clippy bans a second bool parameter.
#[derive(Debug, Clone, Copy)]
pub struct ModerationGate {
    pub should_run: bool,
    pub should_proceed: bool,
}

// Settings::resolve reads APORIO_API_KEY/APORIO_API_BASE, matching LiteLLM's APORIA_* fields.
#[derive(Clone, Deserialize)]
pub struct AporiaGuardrailConfigModel {
    pub api_key: Option<String>,
    pub api_base: Option<String>,
}

impl GuardrailConfigModelIdentity for AporiaGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Aporia AI"
    }
}

fn request_error(source: reqwest::Error) -> AporiaGuardrailError {
    if source.is_timeout() {
        AporiaGuardrailError::Timeout { source }
    } else {
        AporiaGuardrailError::Request { source }
    }
}

async fn read_capped_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, AporiaGuardrailError> {
    let limit_bytes = u64::try_from(limit).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > limit_bytes)
    {
        return Err(AporiaGuardrailError::ResponseTooLarge { limit });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Err(AporiaGuardrailError::ResponseTooLarge { limit });
        }
    }
    Ok(body)
}

#[path = "aporia_guardrail/tests.rs"]
mod tests;
