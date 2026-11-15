use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;
use serde::Deserialize;

use super::error::ThirdPartyInitAError;
use super::{InitResult, decode};

mod tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum GuardrailsAiInputFormat {
    #[serde(rename = "inputs")]
    Inputs,
    #[default]
    #[serde(rename = "llmOutput")]
    LlmOutput,
}

#[derive(Debug, Clone)]
pub struct GuardrailsAiArgs {
    pub name: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub guard_name: String,
    pub guardrails_ai_api_input_format: GuardrailsAiInputFormat,
    pub timeout: Option<f64>,
}

pub fn initialize_guardrails_ai(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<GuardrailsAiArgs> {
    let fields = &params.base.fields;
    let guard_name = fields
        .guard_name
        .clone()
        .ok_or(ThirdPartyInitAError::MissingGuardName)?;
    let format = decode(
        "guardrails_ai_api_input_format",
        params.base.extra.0.get("guardrails_ai_api_input_format"),
    )?;
    Ok(GuardrailsAiArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        api_base: fields.api_base.clone(),
        api_key: fields.api_key.clone(),
        event_hook: params.mode.clone(),
        default_on: fields.default_on.unwrap_or_default(),
        guard_name,
        guardrails_ai_api_input_format: format.unwrap_or_default(),
        timeout: fields.timeout,
    })
}
