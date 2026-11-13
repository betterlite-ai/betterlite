use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

use super::error::AuthLimitInputsError;

// What LiteLLM's int() reads out of one deployment's litellm_params; bytes have no JSON form but a router row can hold them.
#[derive(Clone, Debug, PartialEq)]
pub enum DeploymentLimitValue {
    Integer(i64),
    Unsigned(u64),
    Float(f64),
    Bool(bool),
    Text(String),
    Bytes(Vec<u8>),
    // int() raises TypeError on this shape, and the source swallows it
    Unsupported,
}

impl From<&Json> for DeploymentLimitValue {
    fn from(value: &Json) -> Self {
        match value {
            Json::Number(number) => number_value(number),
            Json::Bool(flag) => Self::Bool(*flag),
            Json::Text(text) => Self::Text(text.clone()),
            Json::Null | Json::List(_) | Json::Object(_) => Self::Unsupported,
        }
    }
}

fn number_value(number: &Number) -> DeploymentLimitValue {
    if let Some(value) = number.as_i64() {
        return DeploymentLimitValue::Integer(value);
    }
    if let Some(value) = number.as_u64() {
        return DeploymentLimitValue::Unsigned(value);
    }
    match number.as_f64() {
        Some(value) => DeploymentLimitValue::Float(value),
        None => DeploymentLimitValue::Unsupported,
    }
}

// The two per-deployment defaults, with absent and null both unset.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeploymentLimits {
    pub default_api_key_rpm_limit: Option<DeploymentLimitValue>,
    pub default_api_key_tpm_limit: Option<DeploymentLimitValue>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeploymentLimit {
    Rpm,
    Tpm,
}

impl DeploymentLimit {
    const fn value_of(self, deployment: &DeploymentLimits) -> Option<&DeploymentLimitValue> {
        match self {
            Self::Rpm => deployment.default_api_key_rpm_limit.as_ref(),
            Self::Tpm => deployment.default_api_key_tpm_limit.as_ref(),
        }
    }
}

// B302's catalog as the limiter reads it; None stands for LiteLLM's llm_router being None.
pub trait DeploymentLimitCatalog {
    fn model_list(&self, model_name: &str) -> Option<Vec<DeploymentLimits>>;
}

pub fn deployment_default_limit(
    catalog: Option<&dyn DeploymentLimitCatalog>,
    model_name: &str,
    limit: DeploymentLimit,
) -> Result<Option<i64>, AuthLimitInputsError> {
    let Some(source) = catalog else {
        return Ok(None);
    };
    let Some(deployments) = source.model_list(model_name) else {
        return Ok(None);
    };
    let mut limits = Vec::new();
    for deployment in deployments {
        let Some(value) = limit.value_of(&deployment) else {
            continue;
        };
        match converted_limit(value) {
            ConvertedLimit::Value(converted) => limits.push(converted),
            ConvertedLimit::Skipped => {}
            ConvertedLimit::Overflow => {
                return Err(AuthLimitInputsError::LimitOverflow {
                    model_name: model_name.to_owned(),
                });
            }
        }
    }
    Ok(limits.into_iter().min())
}

enum ConvertedLimit {
    Value(i64),
    Skipped,
    Overflow,
}

// int() skips a value it raises ValueError or TypeError on, while an OverflowError escapes to the caller.
fn converted_limit(value: &DeploymentLimitValue) -> ConvertedLimit {
    match value {
        DeploymentLimitValue::Integer(number) => ConvertedLimit::Value(*number),
        DeploymentLimitValue::Unsigned(number) => {
            i64::try_from(*number).map_or(ConvertedLimit::Overflow, ConvertedLimit::Value)
        }
        DeploymentLimitValue::Bool(flag) => ConvertedLimit::Value(i64::from(*flag)),
        DeploymentLimitValue::Float(number) => float_limit(*number),
        DeploymentLimitValue::Text(text) => parsed_integer(text),
        DeploymentLimitValue::Bytes(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => parsed_integer(text),
            Err(_) => ConvertedLimit::Skipped,
        },
        DeploymentLimitValue::Unsupported => ConvertedLimit::Skipped,
    }
}

fn float_limit(number: f64) -> ConvertedLimit {
    if number.is_nan() {
        return ConvertedLimit::Skipped;
    }
    if !number.is_finite() {
        return ConvertedLimit::Overflow;
    }
    match Decimal::from_f64_retain(number).map(Decimal::trunc) {
        Some(truncated) => truncated
            .to_i64()
            .map_or(ConvertedLimit::Overflow, ConvertedLimit::Value),
        None => ConvertedLimit::Overflow,
    }
}

// int() takes surrounding whitespace, a sign, decimal digits and single underscores between them
fn parsed_integer(text: &str) -> ConvertedLimit {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    if digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return ConvertedLimit::Skipped;
    }
    match trimmed.replace('_', "").parse::<i64>() {
        Ok(number) => ConvertedLimit::Value(number),
        Err(_) => ConvertedLimit::Skipped,
    }
}
