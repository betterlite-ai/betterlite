use betterlite_config::agentic_loop_ceiling::{
    AgenticLoopCeilingError, AgenticLoopValue, DEFAULT_MAX_AGENTIC_LOOPS,
    validated_max_agentic_loops,
};
use betterlite_logging::analytics::python;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use num_traits::ToPrimitive as _;
use serde_json::Number;

use super::error::AgenticLoopSettingsError;

const CEILING_FIELD: &str = "litellm_params.max_agentic_loops";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgenticLoopSettings {
    pub depth: i64,
    pub max_loops: u32,
    pub fingerprints: Vec<Json>,
}

pub fn get_agentic_loop_settings(
    kwargs: &IndexMap<String, Json>,
) -> Result<AgenticLoopSettings, AgenticLoopSettingsError> {
    let depth = trusted_depth(kwargs.get("_agentic_loop_depth"))?;
    let max_loops = match kwargs.get("max_agentic_loops") {
        Some(value) => agentic_ceiling(value)?,
        None => DEFAULT_MAX_AGENTIC_LOOPS,
    };
    let fingerprints = fingerprint_history(kwargs.get("_agentic_loop_fingerprints"))?;
    Ok(AgenticLoopSettings {
        depth,
        max_loops,
        fingerprints,
    })
}

fn agentic_ceiling(value: &Json) -> Result<u32, AgenticLoopSettingsError> {
    let parsed = match value {
        Json::Null => AgenticLoopValue::Null,
        Json::Bool(flag) => AgenticLoopValue::Boolean(*flag),
        Json::Number(number) => number_loop_value(number),
        Json::Text(text) => AgenticLoopValue::Text(text.clone()),
        // the shared validator has no list or object arm, and neither spells a ceiling
        Json::List(_) | Json::Object(_) => {
            return Err(AgenticLoopSettingsError::CeilingNotInteger {
                literal: python::repr(value)?,
            });
        }
    };
    match validated_max_agentic_loops(&parsed, CEILING_FIELD) {
        Ok(Some(ceiling)) => Ok(ceiling),
        Ok(None) => Ok(DEFAULT_MAX_AGENTIC_LOOPS),
        Err(AgenticLoopCeilingError::BelowMinimum { value: minimum, .. }) => {
            Err(AgenticLoopSettingsError::CeilingBelowMinimum { value: minimum })
        }
        Err(AgenticLoopCeilingError::NotWholeNumber { .. }) => {
            Err(AgenticLoopSettingsError::CeilingNotInteger {
                literal: python::repr(value)?,
            })
        }
    }
}

fn number_loop_value(number: &Number) -> AgenticLoopValue {
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        return match text.parse::<f64>() {
            Ok(value) => AgenticLoopValue::Float(value),
            Err(_) => AgenticLoopValue::Text(text),
        };
    }
    match text.parse::<i64>() {
        Ok(value) => AgenticLoopValue::Integer(value),
        Err(_) => AgenticLoopValue::Text(text),
    }
}

fn trusted_depth(value: Option<&Json>) -> Result<i64, AgenticLoopSettingsError> {
    let Some(value) = value else {
        return Ok(0);
    };
    if is_empty_or_zero(value) {
        return Ok(0);
    }
    match value {
        Json::Bool(true) => Ok(1),
        Json::Number(number) => trusted_number_depth(number),
        Json::Text(text) => trusted_text_depth(text),
        Json::List(_) => Err(AgenticLoopSettingsError::DepthWrongType { type_name: "list" }),
        Json::Object(_) => Err(AgenticLoopSettingsError::DepthWrongType { type_name: "dict" }),
        // every remaining falsy value returned above; the arm keeps the match exhaustive
        Json::Null | Json::Bool(false) => Ok(0),
    }
}

fn is_empty_or_zero(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Bool(flag) => !*flag,
        Json::Number(number) => number_is_zero(number),
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(object) => object.is_empty(),
    }
}

fn number_is_zero(number: &Number) -> bool {
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        let bits = number.as_f64().map(f64::to_bits);
        return matches!(bits, Some(0) | Some(NEGATIVE_ZERO_BITS));
    }
    text.parse::<i64>() == Ok(0)
}

const NEGATIVE_ZERO_BITS: u64 = 0x8000_0000_0000_0000;

fn trusted_number_depth(number: &Number) -> Result<i64, AgenticLoopSettingsError> {
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        return float_depth(&text);
    }
    match text.parse::<i64>() {
        Ok(value) => Ok(value),
        Err(_) => Err(AgenticLoopSettingsError::DepthOutOfRange),
    }
}

fn float_depth(text: &str) -> Result<i64, AgenticLoopSettingsError> {
    let Ok(value) = text.parse::<f64>() else {
        return Err(AgenticLoopSettingsError::DepthOutOfRange);
    };
    if value.is_nan() {
        return Err(AgenticLoopSettingsError::DepthNan);
    }
    if value.is_infinite() {
        return Err(AgenticLoopSettingsError::DepthInfinity);
    }
    value
        .to_i64()
        .ok_or(AgenticLoopSettingsError::DepthOutOfRange)
}

// betterlite_config::agentic_loop_ceiling::whole_number is private, and its ceiling rules reject depths int() accepts
fn trusted_text_depth(text: &str) -> Result<i64, AgenticLoopSettingsError> {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    if !digits.split('_').all(|part| !part.is_empty()) {
        return Err(AgenticLoopSettingsError::DepthInvalidLiteral {
            literal: python::repr(&Json::Text(text.to_owned()))?,
        });
    }
    match trimmed.replace('_', "").parse::<i64>() {
        Ok(value) => Ok(value),
        Err(_) => Err(AgenticLoopSettingsError::DepthInvalidLiteral {
            literal: python::repr(&Json::Text(text.to_owned()))?,
        }),
    }
}

fn fingerprint_history(value: Option<&Json>) -> Result<Vec<Json>, AgenticLoopSettingsError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if is_empty_or_zero(value) {
        return Ok(Vec::new());
    }
    match value {
        Json::List(items) => Ok(items.clone()),
        Json::Text(text) => Ok(text
            .chars()
            .map(|character| Json::Text(character.to_string()))
            .collect()),
        Json::Object(object) => Ok(object.keys().map(|key| Json::Text(key.clone())).collect()),
        Json::Bool(true) => {
            Err(AgenticLoopSettingsError::FingerprintsNotIterable { type_name: "bool" })
        }
        Json::Number(number) => Err(AgenticLoopSettingsError::FingerprintsNotIterable {
            type_name: number_type_name(number),
        }),
        // every remaining value is falsy and returned above
        Json::Null | Json::Bool(false) => Ok(Vec::new()),
    }
}

fn number_type_name(number: &Number) -> &'static str {
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        "float"
    } else {
        "int"
    }
}
