use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use rust_decimal::Decimal;
use serde_json::{Value, from_value, to_value};

use super::error::RecordsError;
use super::options::{decimal_value, integer_value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointingOptions {
    pub start_threshold: Option<Decimal>,
    pub stop_threshold: Option<Decimal>,
    pub stop_history: Option<u32>,
    pub stop_history_eou: Option<u32>,
}

// An explicit endpointing_config is copied verbatim; a server_vad chunking_strategy is translated into these settings
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointingConfig {
    Explicit(BTreeMap<String, Json>),
    Derived(EndpointingOptions),
}

pub(super) fn endpointing_config(
    params: &Object,
) -> Result<Option<EndpointingConfig>, RecordsError> {
    if let Some(Value::Object(explicit)) = params.get("endpointing_config") {
        return explicit_endpointing(explicit).map(Some);
    }
    let Some(Value::Object(chunking)) = params.get("chunking_strategy") else {
        return Ok(None);
    };
    if chunking.get("type").and_then(Value::as_str) != Some("server_vad") {
        return Ok(None);
    }
    derived_endpointing(chunking)
}

fn explicit_endpointing(explicit: &Object) -> Result<EndpointingConfig, RecordsError> {
    let mut entries = BTreeMap::new();
    for (name, value) in explicit {
        let typed = from_value::<Json>(value.clone()).map_err(|source| RecordsError::Value {
            field: "endpointing_config",
            source,
        })?;
        entries.insert(name.clone(), typed);
    }
    Ok(EndpointingConfig::Explicit(entries))
}

fn derived_endpointing(chunking: &Object) -> Result<Option<EndpointingConfig>, RecordsError> {
    let mut endpointing = EndpointingOptions {
        start_threshold: None,
        stop_threshold: None,
        stop_history: None,
        stop_history_eou: None,
    };
    if let Some(threshold) = chunking.get("threshold") {
        let threshold = decimal_value(threshold, "chunking_strategy.threshold")?;
        endpointing.start_threshold = Some(threshold);
        endpointing.stop_threshold = Some(threshold);
    }
    if let Some(silence) = chunking.get("silence_duration_ms") {
        let field = "chunking_strategy.silence_duration_ms";
        endpointing.stop_history = Some(integer_value(silence, field)?);
    }
    if let Some(padding) = chunking.get("prefix_padding_ms") {
        let field = "chunking_strategy.prefix_padding_ms";
        endpointing.stop_history_eou = Some(integer_value(padding, field)?);
    }
    if endpointing.start_threshold.is_none()
        && endpointing.stop_threshold.is_none()
        && endpointing.stop_history.is_none()
        && endpointing.stop_history_eou.is_none()
    {
        return Ok(None);
    }
    Ok(Some(EndpointingConfig::Derived(endpointing)))
}

impl EndpointingConfig {
    pub(super) fn to_value(&self) -> Result<Value, RecordsError> {
        match self {
            Self::Explicit(entries) => explicit_value(entries),
            Self::Derived(options) => derived_value(options),
        }
    }
}

fn explicit_value(entries: &BTreeMap<String, Json>) -> Result<Value, RecordsError> {
    let mut object = Object::new();
    for (name, value) in entries {
        let converted = to_value(value).map_err(|source| RecordsError::Value {
            field: "endpointing_config",
            source,
        })?;
        object.insert(name.clone(), converted);
    }
    Ok(Value::Object(object))
}

fn derived_value(options: &EndpointingOptions) -> Result<Value, RecordsError> {
    let mut object = Object::new();
    if let Some(threshold) = options.start_threshold {
        object.insert("start_threshold".to_owned(), threshold_value(threshold)?);
    }
    if let Some(threshold) = options.stop_threshold {
        object.insert("stop_threshold".to_owned(), threshold_value(threshold)?);
    }
    if let Some(history) = options.stop_history {
        object.insert("stop_history".to_owned(), Value::from(history));
    }
    if let Some(history) = options.stop_history_eou {
        object.insert("stop_history_eou".to_owned(), Value::from(history));
    }
    Ok(Value::Object(object))
}

fn threshold_value(threshold: Decimal) -> Result<Value, RecordsError> {
    threshold
        .to_f64()
        .map(Value::from)
        .ok_or(RecordsError::Number {
            field: "chunking_strategy.threshold",
        })
}
