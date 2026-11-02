use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use url::Url;

pub mod error;
use error::GuardrailsAiGuardrailError;

mod http;
use http::{parse_pre_call_response, read_capped_body, request_error};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const DEFAULT_API_BASE: &str = "http://0.0.0.0:8000";
const COMPLETION_CALL_TYPES: [&str; 2] = ["completion", "acompletion"];

pub const SUPPORTED_EVENT_HOOKS: [EventHook; 3] = [
    EventHook::PostCall,
    EventHook::PreCall,
    EventHook::LoggingOnly,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EventHook {
    #[serde(rename = "post_call")]
    PostCall,
    #[serde(rename = "pre_call")]
    PreCall,
    #[serde(rename = "logging_only")]
    LoggingOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InputFormat {
    Inputs,
    LlmOutput,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub guard_name: String,
    pub api_base: String,
    pub input_format: InputFormat,
}

impl Settings {
    // See docs/deviations/B3753.yaml: LiteLLM raises a bare Exception instead.
    pub fn resolve<L>(
        guard_name: Option<String>,
        api_base: Option<String>,
        input_format: InputFormat,
        lookup: &L,
    ) -> Result<Self, GuardrailsAiGuardrailError>
    where
        L: EnvironmentLookup,
    {
        let resolved_guard_name = guard_name.ok_or(GuardrailsAiGuardrailError::MissingGuardName)?;
        let resolved_api_base = api_base
            .or_else(|| lookup.lookup("GUARDRAILS_AI_API_BASE"))
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        Ok(Self {
            guard_name: resolved_guard_name,
            api_base: resolved_api_base,
            input_format,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardrailsAiApiResponse {
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub raw_llm_output: Option<String>,
    #[serde(default)]
    pub validated_output: Option<String>,
    #[serde(default)]
    pub validation_passed: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InferenceData {
    pub name: String,
    pub shape: Vec<u32>,
    pub data: Vec<String>,
    pub datatype: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GuardrailsAiPreCallApiResponse {
    #[serde(default)]
    pub modelname: Option<String>,
    #[serde(default)]
    pub modelversion: Option<String>,
    #[serde(default)]
    pub outputs: Vec<InferenceData>,
}

#[derive(Debug, Clone, Serialize)]
struct LlmOutputRequest {
    #[serde(rename = "llmOutput")]
    llm_output: String,
    #[serde(flatten)]
    dynamic_params: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Serialize)]
struct PreCallInputData {
    name: String,
    shape: Vec<u32>,
    data: Vec<String>,
    datatype: String,
}

#[derive(Debug, Clone, Serialize)]
struct PreCallRequest {
    inputs: Vec<PreCallInputData>,
}

#[derive(Debug, Clone)]
pub struct GuardrailsAiGuardrail {
    pub settings: Settings,
    pub client: reqwest::Client,
    pub timeout: Duration,
}

impl GuardrailsAiGuardrail {
    // httpx's URL.join follows RFC 3986 merge: no trailing slash drops the base's last segment.
    fn validate_url(&self) -> Result<Url, GuardrailsAiGuardrailError> {
        let base = Url::parse(&self.settings.api_base)
            .map_err(|source| GuardrailsAiGuardrailError::InvalidApiBase { source })?;
        base.join(&format!("guards/{}/validate", self.settings.guard_name))
            .map_err(|source| GuardrailsAiGuardrailError::InvalidApiBase { source })
    }

    pub async fn make_guardrails_ai_api_request(
        &self,
        llm_output: &str,
        dynamic_params: &BTreeMap<String, GuardrailValue>,
    ) -> Result<GuardrailsAiApiResponse, GuardrailsAiGuardrailError> {
        let body = LlmOutputRequest {
            llm_output: llm_output.to_owned(),
            dynamic_params: dynamic_params.clone(),
        };
        let url = self.validate_url()?;
        let response = self
            .client
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(GuardrailsAiGuardrailError::HttpStatus { status });
        }
        let bytes = read_capped_body(response, MAX_RESPONSE_BYTES).await?;
        let parsed: GuardrailsAiApiResponse = serde_json::from_slice(&bytes)
            .map_err(|source| GuardrailsAiGuardrailError::ResponseJson { source })?;
        if parsed.validation_passed == Some(false) {
            return Err(GuardrailsAiGuardrailError::Blocked { response: parsed });
        }
        Ok(parsed)
    }

    // See docs/deviations/B3753.yaml: LiteLLM keeps this branch despite calling it broken.
    pub async fn make_pre_call_request(
        &self,
        text_input: &str,
    ) -> Result<String, GuardrailsAiGuardrailError> {
        let body = PreCallRequest {
            inputs: vec![PreCallInputData {
                name: "text".to_owned(),
                shape: vec![1],
                data: vec![text_input.to_owned()],
                datatype: "BYTES".to_owned(),
            }],
        };
        let url = self.validate_url()?;
        let response = self
            .client
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        let bytes = read_capped_body(response, MAX_RESPONSE_BYTES).await?;
        parse_pre_call_response(status, &bytes)
    }

    pub async fn process_input(
        &self,
        last_user_message: Option<String>,
        dynamic_params: &BTreeMap<String, GuardrailValue>,
        call_type: &str,
    ) -> Result<Option<String>, GuardrailsAiGuardrailError> {
        if !COMPLETION_CALL_TYPES.contains(&call_type) {
            return Ok(last_user_message);
        }
        let Some(text) = last_user_message else {
            return Ok(None);
        };
        let updated_text = match self.settings.input_format {
            InputFormat::Inputs => self.make_pre_call_request(&text).await?,
            InputFormat::LlmOutput => {
                let result = self
                    .make_guardrails_ai_api_request(&text, dynamic_params)
                    .await?;
                result
                    .validated_output
                    .filter(|value| !value.is_empty())
                    .or_else(|| result.raw_llm_output.filter(|value| !value.is_empty()))
                    .unwrap_or(text)
            }
        };
        Ok(Some(updated_text))
    }

    pub async fn async_logging_hook(
        &self,
        last_user_message: Option<String>,
        dynamic_params: &BTreeMap<String, GuardrailValue>,
        call_type: &str,
    ) -> Result<Option<String>, GuardrailsAiGuardrailError> {
        if !COMPLETION_CALL_TYPES.contains(&call_type) {
            return Ok(last_user_message);
        }
        self.process_input(last_user_message, dynamic_params, call_type)
            .await
    }

    // Returns whether the guardrail ran, for the (out-of-scope) applied-guardrails header.
    pub async fn async_post_call_success_hook(
        &self,
        should_run: bool,
        response_content: Option<String>,
        dynamic_params: &BTreeMap<String, GuardrailValue>,
    ) -> Result<bool, GuardrailsAiGuardrailError> {
        if !should_run {
            return Ok(false);
        }
        let Some(content) = response_content.filter(|value| !value.is_empty()) else {
            return Ok(false);
        };
        self.make_guardrails_ai_api_request(&content, dynamic_params)
            .await?;
        Ok(true)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GuardrailsAiConfigModel {
    pub api_base: Option<String>,
    pub guard_name: Option<String>,
    #[serde(default = "default_input_format")]
    pub guardrails_ai_api_input_format: Option<InputFormat>,
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "the field is Option<InputFormat> so an explicit null deserializes to None; this default must match"
)]
const fn default_input_format() -> Option<InputFormat> {
    Some(InputFormat::LlmOutput)
}

impl GuardrailConfigModelIdentity for GuardrailsAiConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Guardrails AI"
    }
}

#[path = "guardrails_ai_guardrail/tests.rs"]
mod tests;
