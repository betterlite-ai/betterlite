use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

pub mod error;
pub mod json_safe;
pub mod wire;

mod enforcement;
mod transport;

use crate::generic_guardrail_contracts::{GuardrailInputType, UnreachableFallback};
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use enforcement::{apply_replacements, blocked, has_selectable_input};
use error::AliceError;
use json_safe::CREDENTIAL_KEYS_TO_STRIP;
use transport::{read_answer, verdict_from_body};
use wire::{
    AliceVerdict, EvaluateRequest, VERDICT_ALLOW, VERDICT_BLOCK, VERDICT_DETECT, VERDICT_MASK,
};

pub const GUARDRAIL_NAME: &str = "alice";
pub const DEFAULT_API_BASE: &str = "https://api.alice.io";
pub const EVALUATE_PATH: &str = "/v2/evaluate/litellm";
pub const DEFAULT_BLOCK_MESSAGE: &str = "Blocked by your organization's content policy.";
// LiteLLM falls back to httpx's 600 second read/write timeout when no guardrail timeout is configured
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AliceConfigModel {
    #[serde(default)]
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub api_base: Option<String>,
}

impl GuardrailConfigModelIdentity for AliceConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Alice"
    }
}

#[derive(Debug, Default)]
pub struct AliceGuardrailParams {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub unreachable_fallback: Option<UnreachableFallback>,
}

#[derive(Debug)]
pub struct AliceGuardrail {
    api_key: SecretString,
    evaluate_url: String,
    unreachable_fallback: UnreachableFallback,
    client: reqwest::Client,
    timeout: Duration,
}

impl AliceGuardrail {
    pub fn new(
        params: AliceGuardrailParams,
        get_secret_str: fn(&str) -> String,
        client: reqwest::Client,
        timeout: Duration,
    ) -> Result<Self, AliceError> {
        let configured_key = params.api_key.filter(|key| !key.expose_secret().is_empty());
        let api_key = match configured_key {
            Some(key) => key,
            None => {
                let from_env = get_secret_str("ALICE_API_KEY");
                if from_env.is_empty() {
                    return Err(AliceError::MissingSecrets);
                }
                SecretString::from(from_env)
            }
        };
        let base = match params.api_base.filter(|base| !base.is_empty()) {
            Some(base) => base,
            None => {
                let from_env = get_secret_str("ALICE_API_BASE");
                if from_env.is_empty() {
                    DEFAULT_API_BASE.to_owned()
                } else {
                    from_env
                }
            }
        };
        Ok(Self {
            api_key,
            evaluate_url: format!("{}{EVALUATE_PATH}", base.trim_end_matches('/')),
            unreachable_fallback: params
                .unreachable_fallback
                .unwrap_or(UnreachableFallback::FailClosed),
            client,
            timeout,
        })
    }

    fn payload(
        inputs: &GuardrailApiInputs,
        request_data: &BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
    ) -> Result<EvaluateRequest, AliceError> {
        let bounded = serde_json::to_value(inputs)
            .map_err(|source| AliceError::Payload { source })
            .and_then(|value| {
                serde_json::from_value::<GuardrailValue>(value)
                    .map_err(|source| AliceError::Payload { source })
            })?;
        let mut seen = Vec::new();
        Ok(EvaluateRequest {
            input_type,
            inputs: json_safe::json_safe(&bounded, 0, &[], &mut seen),
            request_data: json_safe::json_safe(
                &GuardrailValue::Object(request_data.clone()),
                0,
                &CREDENTIAL_KEYS_TO_STRIP,
                &mut seen,
            ),
        })
    }

    fn headers(&self) -> Result<HeaderMap, AliceError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let mut api_key = HeaderValue::from_str(self.api_key.expose_secret())
            .map_err(|source| AliceError::InvalidApiKeyHeader { source })?;
        api_key.set_sensitive(true);
        headers.insert(HeaderName::from_static("af-api-key"), api_key);
        Ok(headers)
    }

    async fn evaluate(
        &self,
        inputs: &GuardrailApiInputs,
        request_data: &BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
    ) -> Result<AliceVerdict, AliceError> {
        let request = Self::payload(inputs, request_data, input_type)?;
        let response = self
            .client
            .post(&self.evaluate_url)
            .headers(self.headers()?)
            .json(&request)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(error::request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(AliceError::Status { status });
        }
        let body = read_answer(response).await?;
        verdict_from_body(&body)
    }

    fn enforce(
        &self,
        verdict: &AliceVerdict,
        mut inputs: GuardrailApiInputs,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        match verdict.verdict.as_deref() {
            Some(VERDICT_ALLOW) => Ok(inputs),
            Some(VERDICT_BLOCK) => Err(blocked(verdict)),
            Some(VERDICT_DETECT) => {
                tracing::warn!(
                    correlation_id = ?verdict.correlation_id,
                    categories = ?verdict.categories,
                    "Alice guardrail: detection recorded, request allowed"
                );
                Ok(inputs)
            }
            Some(VERDICT_MASK) => {
                apply_replacements(verdict, &mut inputs)?;
                Ok(inputs)
            }
            unrecognized => self.on_unreachable(
                &AliceError::UnrecognizedVerdict {
                    verdict: unrecognized.map(str::to_owned),
                },
                inputs,
            ),
        }
    }

    fn on_unreachable(
        &self,
        error: &AliceError,
        inputs: GuardrailApiInputs,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        if self.unreachable_fallback == UnreachableFallback::FailOpen {
            tracing::error!(
                error = %error,
                "Alice guardrail unreachable, allowing request per unreachable_fallback"
            );
            return Ok(inputs);
        }
        Err(GuardrailFailure::GuardrailRaised {
            guardrail_name: Some(GUARDRAIL_NAME.to_owned()),
            message: "Alice guardrail is unavailable and this request cannot be checked".to_owned(),
            wrap_default_message: false,
            status_code: 400,
            blocked_content: false,
        })
    }
}

impl GuardrailContract for AliceGuardrail {
    async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        if !has_selectable_input(&inputs) {
            return Ok(inputs);
        }
        let verdict = match self.evaluate(&inputs, request_data, input_type).await {
            Ok(verdict) => verdict,
            Err(error @ AliceError::Status { status }) if status.is_server_error() => {
                return self.on_unreachable(&error, inputs);
            }
            Err(AliceError::Status { status }) => {
                return Err(GuardrailFailure::Http {
                    status: status.as_u16(),
                    detail: String::new(),
                });
            }
            Err(error) => return self.on_unreachable(&error, inputs),
        };
        self.enforce(&verdict, inputs)
    }
}

mod tests;
