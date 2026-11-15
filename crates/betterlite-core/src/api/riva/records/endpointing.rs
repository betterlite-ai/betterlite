use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::from_value;

use super::error::RecordsError;
use super::options::{decimal_at, integer_at};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EndpointingOptions {
    #[serde(
        with = "rust_decimal::serde::float_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub start_threshold: Option<Decimal>,
    #[serde(
        with = "rust_decimal::serde::float_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub stop_threshold: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_history: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_history_eou: Option<u32>,
}

// An explicit endpointing_config is copied verbatim; a server_vad chunking_strategy is translated into these settings
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum EndpointingConfig {
    Explicit(BTreeMap<String, Json>),
    Derived(EndpointingOptions),
}

pub(super) fn endpointing_config(
    params: &Object,
) -> Result<Option<EndpointingConfig>, RecordsError> {
    if let Some(explicit) = params
        .get("endpointing_config")
        .and_then(|value| value.as_object())
    {
        return explicit_endpointing(explicit).map(Some);
    }
    let Some(chunking) = params
        .get("chunking_strategy")
        .and_then(|value| value.as_object())
    else {
        return Ok(None);
    };
    if chunking.get("type").and_then(|value| value.as_str()) != Some("server_vad") {
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
    if let Some(threshold) = decimal_at(chunking, "threshold", "chunking_strategy.threshold")? {
        endpointing.start_threshold = Some(threshold);
        endpointing.stop_threshold = Some(threshold);
    }
    endpointing.stop_history = integer_at(
        chunking,
        "silence_duration_ms",
        "chunking_strategy.silence_duration_ms",
    )?;
    endpointing.stop_history_eou = integer_at(
        chunking,
        "prefix_padding_ms",
        "chunking_strategy.prefix_padding_ms",
    )?;
    if endpointing.start_threshold.is_none()
        && endpointing.stop_threshold.is_none()
        && endpointing.stop_history.is_none()
        && endpointing.stop_history_eou.is_none()
    {
        return Ok(None);
    }
    Ok(Some(EndpointingConfig::Derived(endpointing)))
}
