use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use thiserror::Error;

mod tests;

#[derive(Debug, Error)]
pub enum QualifireInitError {
    #[error("the Qualifire guardrail parameter {} has an invalid value", .0)]
    InvalidParam(String),
}

#[derive(Debug, Clone)]
pub struct QualifireArgs {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub evaluation_id: Option<String>,
    pub prompt_injections: Option<bool>,
    pub hallucinations_check: Option<bool>,
    pub grounding_check: Option<bool>,
    pub pii_check: Option<bool>,
    pub content_moderation_check: Option<bool>,
    pub tool_selection_quality_check: Option<bool>,
    pub assertions: Option<Vec<String>>,
    pub on_flagged: Option<String>,
    pub name: String,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
}

pub fn initialize_qualifire(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<QualifireArgs, QualifireInitError> {
    let extra = &params.base.extra.0;
    Ok(QualifireArgs {
        api_key: params.base.fields.api_key.clone(),
        api_base: params.base.fields.api_base.clone(),
        evaluation_id: read(extra, "evaluation_id", None)?,
        prompt_injections: read(extra, "prompt_injections", None)?,
        hallucinations_check: read(extra, "hallucinations_check", None)?,
        grounding_check: read(extra, "grounding_check", None)?,
        pii_check: read(extra, "pii_check", None)?,
        content_moderation_check: read(extra, "content_moderation_check", None)?,
        tool_selection_quality_check: read(extra, "tool_selection_quality_check", None)?,
        assertions: read(extra, "assertions", None)?,
        on_flagged: read(extra, "on_flagged", Some("block".to_owned()))?,
        name: guardrail_name.unwrap_or_default().to_owned(),
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
        timeout: params.base.fields.timeout,
    })
}

// LiteLLM's fields default to None, a null stays None and an absent key takes the given default
pub(super) fn read<T>(
    extra: &BTreeMap<String, GuardrailValue>,
    key: &str,
    default: Option<T>,
) -> Result<Option<T>, QualifireInitError>
where
    T: DeserializeOwned,
{
    match extra.get(key) {
        None => Ok(default),
        Some(GuardrailValue::Null) => Ok(None),
        Some(value) => {
            let encoded = serde_json::to_vec(value)
                .map_err(|_not_json| QualifireInitError::InvalidParam(key.to_owned()))?;
            serde_json::from_slice(&encoded)
                .map(Some)
                .map_err(|_wrong_type| QualifireInitError::InvalidParam(key.to_owned()))
        }
    }
}
