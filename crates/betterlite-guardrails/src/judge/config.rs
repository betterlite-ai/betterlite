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

pub(crate) trait ParamKind: Clone {
    fn from_value(value: &GuardrailValue) -> Option<Self>;
    fn from_attributes(params: &JudgeParams, key: ParamKey) -> Option<Self>;
}

impl ParamKind for String {
    fn from_value(value: &GuardrailValue) -> Option<Self> {
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

    fn from_attributes(params: &JudgeParams, key: ParamKey) -> Option<Self> {
        match key {
            ParamKey::JudgeModel => params.judge_model.clone(),
            ParamKey::OnFailure => params.on_failure.clone(),
            ParamKey::Criteria
            | ParamKey::OverallThreshold
            | ParamKey::Mode
            | ParamKey::DefaultOn => None,
        }
    }
}

impl ParamKind for Vec<Criterion> {
    fn from_value(value: &GuardrailValue) -> Option<Self> {
        let encoded = serde_json::to_string(value).ok()?;
        serde_json::from_str(&encoded).ok()
    }

    fn from_attributes(params: &JudgeParams, key: ParamKey) -> Option<Self> {
        match key {
            ParamKey::Criteria => params.criteria.clone(),
            ParamKey::JudgeModel
            | ParamKey::OverallThreshold
            | ParamKey::OnFailure
            | ParamKey::Mode
            | ParamKey::DefaultOn => None,
        }
    }
}

impl ParamKind for f64 {
    fn from_value(value: &GuardrailValue) -> Option<Self> {
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

    fn from_attributes(params: &JudgeParams, key: ParamKey) -> Option<Self> {
        match key {
            ParamKey::OverallThreshold => params.overall_threshold,
            ParamKey::JudgeModel
            | ParamKey::Criteria
            | ParamKey::OnFailure
            | ParamKey::Mode
            | ParamKey::DefaultOn => None,
        }
    }
}

impl ParamKind for bool {
    fn from_value(value: &GuardrailValue) -> Option<Self> {
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

    fn from_attributes(params: &JudgeParams, key: ParamKey) -> Option<Self> {
        match key {
            ParamKey::DefaultOn => params.default_on,
            ParamKey::JudgeModel
            | ParamKey::Criteria
            | ParamKey::OverallThreshold
            | ParamKey::OnFailure
            | ParamKey::Mode => None,
        }
    }
}

impl ParamKind for EventHookParameter {
    fn from_value(value: &GuardrailValue) -> Option<Self> {
        let encoded = serde_json::to_string(value).ok()?;
        serde_json::from_str(&encoded).ok()
    }

    fn from_attributes(params: &JudgeParams, key: ParamKey) -> Option<Self> {
        match key {
            ParamKey::Mode => params.mode.clone(),
            ParamKey::JudgeModel
            | ParamKey::Criteria
            | ParamKey::OverallThreshold
            | ParamKey::OnFailure
            | ParamKey::DefaultOn => None,
        }
    }
}

pub(crate) fn get_param<T>(
    typed: Option<&T>,
    guardrail: &GuardrailEntry<'_>,
    key: ParamKey,
) -> Result<Option<T>, JudgeError>
where
    T: ParamKind,
{
    if let Some(value) = typed {
        return Ok(Some(value.clone()));
    }
    let Some(source) = &guardrail.litellm_params else {
        return Ok(None);
    };
    match source {
        LitellmParamsSource::Mapping(mapping) => mapping.get(key.name()).map_or_else(
            || Ok(None),
            |raw| {
                T::from_value(raw)
                    .map(Some)
                    .ok_or(JudgeError::InvalidParam { key: key.name() })
            },
        ),
        LitellmParamsSource::Parsed(params) => Ok(T::from_attributes(params, key)),
    }
}
