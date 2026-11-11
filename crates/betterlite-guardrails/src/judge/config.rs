use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_judge::{Criterion, EventHookParameter, JudgeParams};
use rust_decimal::Decimal;

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

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ParamValue {
    Text(String),
    Number(f64),
    Boolean(bool),
    Criteria(Vec<Criterion>),
    EventHook(EventHookParameter),
}

fn attribute(params: &JudgeParams, key: ParamKey) -> Option<ParamValue> {
    match key {
        ParamKey::JudgeModel => params.judge_model.clone().map(ParamValue::Text),
        ParamKey::Criteria => params.criteria.clone().map(ParamValue::Criteria),
        ParamKey::OverallThreshold => params.overall_threshold.map(ParamValue::Number),
        ParamKey::OnFailure => params.on_failure.clone().map(ParamValue::Text),
        ParamKey::Mode => params.mode.clone().map(ParamValue::EventHook),
        ParamKey::DefaultOn => params.default_on.map(ParamValue::Boolean),
    }
}

fn parse_value(key: ParamKey, value: &GuardrailValue) -> Option<ParamValue> {
    match key {
        ParamKey::JudgeModel | ParamKey::OnFailure => text_value(value).map(ParamValue::Text),
        ParamKey::Criteria => {
            serde_json::from_str::<Vec<Criterion>>(&serde_json::to_string(value).ok()?)
                .ok()
                .map(ParamValue::Criteria)
        }
        ParamKey::OverallThreshold => number_value(value).map(ParamValue::Number),
        ParamKey::Mode => {
            serde_json::from_str::<EventHookParameter>(&serde_json::to_string(value).ok()?)
                .ok()
                .map(ParamValue::EventHook)
        }
        ParamKey::DefaultOn => boolean_value(value).map(ParamValue::Boolean),
    }
}

fn text_value(value: &GuardrailValue) -> Option<String> {
    match value {
        GuardrailValue::Text(text) => Some(text.clone()),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => None,
    }
}

fn number_value(value: &GuardrailValue) -> Option<f64> {
    match value {
        GuardrailValue::SignedInteger(integer) => Decimal::from(*integer).try_into().ok(),
        GuardrailValue::UnsignedInteger(integer) => Decimal::from(*integer).try_into().ok(),
        GuardrailValue::Decimal(float) => Some(*float),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => None,
    }
}

const fn boolean_value(value: &GuardrailValue) -> Option<bool> {
    match value {
        GuardrailValue::Boolean(flag) => Some(*flag),
        GuardrailValue::Null
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => None,
    }
}

pub(crate) fn get_param(
    params: &JudgeParams,
    guardrail: &GuardrailEntry<'_>,
    key: ParamKey,
) -> Result<Option<ParamValue>, JudgeError> {
    if let Some(value) = attribute(params, key) {
        return Ok(Some(value));
    }
    let Some(source) = &guardrail.litellm_params else {
        return Ok(None);
    };
    match source {
        LitellmParamsSource::Mapping(mapping) => mapping.get(key.name()).map_or_else(
            || Ok(None),
            |raw| {
                parse_value(key, raw)
                    .map(Some)
                    .ok_or(JudgeError::InvalidParam { key: key.name() })
            },
        ),
        LitellmParamsSource::Parsed(parsed) => Ok(attribute(parsed, key)),
    }
}

pub(crate) fn text_param(value: Option<ParamValue>) -> Option<String> {
    match value {
        Some(ParamValue::Text(text)) => Some(text),
        Some(
            ParamValue::Number(_)
            | ParamValue::Boolean(_)
            | ParamValue::Criteria(_)
            | ParamValue::EventHook(_),
        )
        | None => None,
    }
}

pub(crate) const fn number_param(value: Option<&ParamValue>) -> Option<f64> {
    match value {
        Some(ParamValue::Number(number)) => Some(*number),
        Some(
            ParamValue::Text(_)
            | ParamValue::Boolean(_)
            | ParamValue::Criteria(_)
            | ParamValue::EventHook(_),
        )
        | None => None,
    }
}

pub(crate) const fn boolean_param(value: Option<&ParamValue>) -> Option<bool> {
    match value {
        Some(ParamValue::Boolean(flag)) => Some(*flag),
        Some(
            ParamValue::Text(_)
            | ParamValue::Number(_)
            | ParamValue::Criteria(_)
            | ParamValue::EventHook(_),
        )
        | None => None,
    }
}

pub(crate) fn criteria_param(value: Option<ParamValue>) -> Option<Vec<Criterion>> {
    match value {
        Some(ParamValue::Criteria(criteria)) => Some(criteria),
        Some(
            ParamValue::Text(_)
            | ParamValue::Number(_)
            | ParamValue::Boolean(_)
            | ParamValue::EventHook(_),
        )
        | None => None,
    }
}

pub(crate) fn hook_param(value: Option<ParamValue>) -> Option<EventHookParameter> {
    match value {
        Some(ParamValue::EventHook(hook)) => Some(hook),
        Some(
            ParamValue::Text(_)
            | ParamValue::Number(_)
            | ParamValue::Boolean(_)
            | ParamValue::Criteria(_),
        )
        | None => None,
    }
}
