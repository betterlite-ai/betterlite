use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;
use thiserror::Error;

use crate::logging_state::LoggingState;

pub const DURATION_KEY: &str = "llm_api_duration_ms";
pub const TIMING_WINDOWS_KEY: &str = "llm_api_timing_windows";

const SPEND_METADATA_MARKER: &str = "user_api_key";

#[derive(Debug, Error)]
pub enum CallDurationError {
    #[error("the call duration does not fit a signed 64-bit microsecond count")]
    DurationOverflow,
    #[error("the call details field {field} must be a JSON object")]
    FieldShape { field: &'static str },
    #[error("the measured call duration is not a JSON number")]
    NumberShape,
    #[error("the recorded timing window at position {index} is not a pair of numbers")]
    WindowValue { index: usize },
}

// LiteLLM's _set_duration_in_model_call_details: the duration in ms plus this call's timing window.
pub fn record_call_duration(
    state: &mut LoggingState,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
) -> Result<(), CallDurationError> {
    let elapsed = end_time.signed_duration_since(start_time);
    let micros = elapsed
        .num_microseconds()
        .ok_or(CallDurationError::DurationOverflow)?;
    let millis = scaled_number(micros, 1_000)?;
    state
        .model_call_details
        .insert(DURATION_KEY.to_owned(), millis);
    match state.model_call_details.get_mut("litellm_params") {
        None | Some(Json::Null) => Ok(()),
        Some(Json::Object(params)) => write_windows(params, start_time, end_time),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(CallDurationError::FieldShape {
                field: "litellm_params",
            })
        }
    }
}

fn write_windows(
    params: &mut IndexMap<String, Json>,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
) -> Result<(), CallDurationError> {
    let [metadata, litellm_metadata] = params.get_disjoint_mut(["metadata", "litellm_metadata"]);
    let metadata = object_slot(metadata, "metadata")?;
    let litellm_metadata = object_slot(litellm_metadata, "litellm_metadata")?;
    if let Some(litellm_metadata) = litellm_metadata.filter(|slots| !slots.is_empty()) {
        copy_spend_keys(metadata, litellm_metadata);
        return append_window(litellm_metadata, start_time, end_time);
    }
    let Some(metadata) = metadata.filter(|slots| !slots.is_empty()) else {
        return Ok(());
    };
    append_window(metadata, start_time, end_time)
}

fn copy_spend_keys(
    metadata: Option<&mut IndexMap<String, Json>>,
    litellm_metadata: &mut IndexMap<String, Json>,
) {
    let Some(metadata) = metadata else {
        return;
    };
    for (name, value) in metadata {
        if name.contains(SPEND_METADATA_MARKER) {
            litellm_metadata.insert(name.clone(), value.clone());
        }
    }
}

fn object_slot<'slot>(
    slot: Option<&'slot mut Json>,
    field: &'static str,
) -> Result<Option<&'slot mut IndexMap<String, Json>>, CallDurationError> {
    match slot {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(metadata)) => Ok(Some(metadata)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(CallDurationError::FieldShape { field })
        }
    }
}

fn append_window(
    metadata: &mut IndexMap<String, Json>,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
) -> Result<(), CallDurationError> {
    let mut windows = earlier_windows(metadata.get(TIMING_WINDOWS_KEY))?;
    windows.push(window_pair(start_time, end_time)?);
    metadata.insert(TIMING_WINDOWS_KEY.to_owned(), Json::List(windows));
    Ok(())
}

fn earlier_windows(recorded: Option<&Json>) -> Result<Vec<Json>, CallDurationError> {
    let Some(Json::List(windows)) = recorded else {
        return Ok(Vec::new());
    };
    let mut earlier = Vec::with_capacity(windows.len());
    for (index, window) in windows.iter().enumerate() {
        let Json::List(pair) = window else {
            continue;
        };
        let [first, second] = pair.as_slice() else {
            continue;
        };
        let (Some(first), Some(second)) =
            (window_number(first, index)?, window_number(second, index)?)
        else {
            continue;
        };
        earlier.push(Json::List(vec![first, second]));
    }
    Ok(earlier)
}

fn window_number(value: &Json, index: usize) -> Result<Option<Json>, CallDurationError> {
    match value {
        Json::Number(number) if number.as_f64().is_some_and(f64::is_finite) => {
            Ok(Some(Json::Number(number.clone())))
        }
        Json::Number(_) => Err(CallDurationError::WindowValue { index }),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => Ok(None),
    }
}

fn window_pair(
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
) -> Result<Json, CallDurationError> {
    Ok(Json::List(vec![
        scaled_number(start_time.timestamp_micros(), 1_000_000)?,
        scaled_number(end_time.timestamp_micros(), 1_000_000)?,
    ]))
}

fn scaled_number(units: i64, per: i64) -> Result<Json, CallDurationError> {
    decimal_number(Decimal::from(units) / Decimal::from(per))
}

fn decimal_number(value: Decimal) -> Result<Json, CallDurationError> {
    let text = value.normalize().to_string();
    Number::from_str(&text)
        .map(Json::Number)
        .map_err(|_error| CallDurationError::NumberShape)
}
