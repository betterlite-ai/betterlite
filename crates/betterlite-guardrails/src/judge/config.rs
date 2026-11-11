use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_judge::JudgeParams;
use serde::de::DeserializeOwned;

use super::error::JudgeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKey {
    JudgeModel,
    Criteria,
    OverallThreshold,
    OnFailure,
    Mode,
    DefaultOn,
}

impl ParamKey {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::JudgeModel => "judge_model",
            Self::Criteria => "criteria",
            Self::OverallThreshold => "overall_threshold",
            Self::OnFailure => "on_failure",
            Self::Mode => "mode",
            Self::DefaultOn => "default_on",
        }
    }
}

// LiteLLM reads a guardrail's litellm_params as the parsed object or as the raw mapping the config carried.
#[derive(Debug, Clone, Copy)]
pub enum LitellmParamsSource<'params> {
    Mapping(&'params BTreeMap<String, GuardrailValue>),
    Parsed(&'params JudgeParams),
}

#[derive(Debug, Clone, Copy)]
pub struct GuardrailEntry<'guardrail> {
    pub guardrail_name: Option<&'guardrail str>,
    pub litellm_params: Option<LitellmParamsSource<'guardrail>>,
}

fn attribute(params: &JudgeParams, key: ParamKey) -> Option<GuardrailValue> {
    let encoded = serde_json::to_string(params).ok()?;
    let GuardrailValue::Object(fields) = serde_json::from_str(&encoded).ok()? else {
        return None;
    };
    fields.get(key.name()).cloned()
}

pub(crate) fn get_param(
    params: &JudgeParams,
    guardrail: &GuardrailEntry<'_>,
    key: ParamKey,
) -> Option<GuardrailValue> {
    if let Some(value) = attribute(params, key) {
        return Some(value);
    }
    match &guardrail.litellm_params {
        Some(LitellmParamsSource::Mapping(mapping)) => mapping.get(key.name()).cloned(),
        Some(LitellmParamsSource::Parsed(parsed)) => attribute(parsed, key),
        None => None,
    }
}

pub(crate) fn parse_param<T>(
    params: &JudgeParams,
    guardrail: &GuardrailEntry<'_>,
    key: ParamKey,
) -> Result<Option<T>, JudgeError>
where
    T: DeserializeOwned,
{
    let Some(value) = get_param(params, guardrail, key) else {
        return Ok(None);
    };
    let invalid = || JudgeError::InvalidParam { key: key.name() };
    let encoded = serde_json::to_string(&value).map_err(|_error| invalid())?;
    serde_json::from_str(&encoded)
        .map(Some)
        .map_err(|_error| invalid())
}
