use betterlite_config::agentic_loop_ceiling::{
    AgenticLoopCeilingError, AgenticLoopValue, DEFAULT_MAX_AGENTIC_LOOPS,
    validated_max_agentic_loops,
};
use betterlite_context::agentic_loop_plan::JsonMap;
use betterlite_models::decisions::Json;
use betterlite_openai::code_execution_records::CONVERTED_STREAM_KEY;
use serde_json::Number;
use serde_json::from_str;
use serde_json::value::RawValue;
use thiserror::Error;

use super::safety::sorted_json;
use super::{
    AGENTIC_LOOP_DEPTH_KEY, AGENTIC_LOOP_FINGERPRINTS_KEY, HEADROOM_CONVERTED_STREAM_KEY,
    MAX_AGENTIC_LOOPS_KEY,
};

mod tests;

const MAX_LOOPS_FIELD: &str = "litellm_params.max_agentic_loops";

#[derive(Debug, PartialEq, Eq)]
pub struct AgenticSettings {
    pub depth: u32,
    pub max_loops: u32,
    pub fingerprints: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SettingsError {
    #[error("_agentic_loop_depth must be a whole number: {value}")]
    NotNumber { value: String },
    #[error("_agentic_loop_depth is out of range: {value}")]
    OutOfRange { value: String },
    #[error("could not render the agentic loop fingerprint: {0}")]
    Fingerprint(String),
    #[error(transparent)]
    Ceiling(#[from] AgenticLoopCeilingError),
}

pub(crate) fn read_settings(kwargs: &JsonMap) -> Result<AgenticSettings, SettingsError> {
    Ok(AgenticSettings {
        depth: read_depth(kwargs)?,
        max_loops: read_max_loops(kwargs)?,
        fingerprints: read_fingerprints(kwargs)?,
    })
}

pub(crate) fn converted_stream_requested(kwargs: &JsonMap) -> bool {
    [CONVERTED_STREAM_KEY, HEADROOM_CONVERTED_STREAM_KEY]
        .into_iter()
        .any(|key| is_set(kwargs, key))
}

// LiteLLM's _coerce_int: an int is used, an int(...) string is parsed, every other value means zero
fn read_depth(kwargs: &JsonMap) -> Result<u32, SettingsError> {
    let Some(raw) = kwargs.get(AGENTIC_LOOP_DEPTH_KEY) else {
        return Ok(0);
    };
    let source = raw.get();
    if let Ok(text) = from_str::<String>(source) {
        let number = text
            .parse::<i64>()
            .map_err(|_parse_error| SettingsError::NotNumber {
                value: text.clone(),
            })?;
        return bounded_depth(number, &text);
    }
    match from_str::<Json>(source) {
        Ok(Json::Number(number)) => number_depth(&number, source),
        _ => Ok(0),
    }
}

// a JSON float is a Python float, which _coerce_int leaves at the default; an integer literal past i64 cannot be a valid depth
fn number_depth(number: &Number, source: &str) -> Result<u32, SettingsError> {
    match number.as_i64() {
        Some(integer) => bounded_depth(integer, source),
        None if source.contains(['.', 'e', 'E']) => Ok(0),
        None => Err(SettingsError::OutOfRange {
            value: source.to_owned(),
        }),
    }
}

fn read_max_loops(kwargs: &JsonMap) -> Result<u32, SettingsError> {
    let Some(raw) = kwargs.get(MAX_AGENTIC_LOOPS_KEY) else {
        return Ok(DEFAULT_MAX_AGENTIC_LOOPS);
    };
    let configured = validated_max_agentic_loops(&ceiling_value(raw)?, MAX_LOOPS_FIELD)?;
    Ok(configured.unwrap_or(DEFAULT_MAX_AGENTIC_LOOPS))
}

fn read_fingerprints(kwargs: &JsonMap) -> Result<Vec<String>, SettingsError> {
    let Some(raw) = kwargs.get(AGENTIC_LOOP_FINGERPRINTS_KEY) else {
        return Ok(Vec::new());
    };
    match from_str::<Json>(raw.get()) {
        Ok(Json::List(items)) => items.iter().map(fingerprint_text).collect(),
        _ => Ok(Vec::new()),
    }
}

// Python's str(fp) keeps a string as it is and writes every other value as text
fn fingerprint_text(value: &Json) -> Result<String, SettingsError> {
    match value {
        Json::Text(text) => Ok(text.clone()),
        other => serde_json::to_string(&sorted_json(other))
            .map_err(|error| SettingsError::Fingerprint(error.to_string())),
    }
}

// B4110's validator takes a typed value, so the raw kwarg is mapped onto its five shapes
fn ceiling_value(raw: &RawValue) -> Result<AgenticLoopValue, AgenticLoopCeilingError> {
    let not_whole = || AgenticLoopCeilingError::NotWholeNumber {
        field: MAX_LOOPS_FIELD.to_owned(),
    };
    match from_str::<Json>(raw.get()) {
        Ok(Json::Null) => Ok(AgenticLoopValue::Null),
        Ok(Json::Bool(flag)) => Ok(AgenticLoopValue::Boolean(flag)),
        Ok(Json::Number(number)) => match number.as_i64() {
            Some(integer) => Ok(AgenticLoopValue::Integer(integer)),
            None => number
                .as_f64()
                .map(AgenticLoopValue::Float)
                .ok_or_else(not_whole),
        },
        Ok(Json::Text(text)) => Ok(AgenticLoopValue::Text(text)),
        Ok(Json::List(_) | Json::Object(_)) | Err(_) => Err(not_whole()),
    }
}

fn bounded_depth(number: i64, value: &str) -> Result<u32, SettingsError> {
    u32::try_from(number).map_err(|_conversion_error| SettingsError::OutOfRange {
        value: value.to_owned(),
    })
}

// LiteLLM tests these flags with bool(...), so every JSON shape that Python calls truthy counts
fn is_set(kwargs: &JsonMap, key: &str) -> bool {
    let Some(raw) = kwargs.get(key) else {
        return false;
    };
    match from_str::<Json>(raw.get()) {
        Ok(Json::Null) => false,
        Ok(Json::Bool(flag)) => flag,
        Ok(Json::Number(number)) => number.as_f64().is_some_and(|float| float != 0.0_f64),
        Ok(Json::Text(text)) => !text.is_empty(),
        Ok(Json::List(items)) => !items.is_empty(),
        Ok(Json::Object(entries)) => !entries.is_empty(),
        Err(_) => false,
    }
}
