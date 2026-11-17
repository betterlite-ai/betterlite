use betterlite_openai::base_helpers::{ThinkingRequest, thinking_request as base_thinking_request};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::converse_params::error::ConverseParamsError;
use crate::model_routes::{ParamMap, ParamValue};

// The prerequisites read their own JSON records, so a typed value crosses that boundary and comes back.
pub fn transcode<T, U>(value: &U) -> Result<T, ConverseParamsError>
where
    T: DeserializeOwned,
    U: Serialize + ?Sized,
{
    let encoded = serde_json::to_value(value).map_err(ConverseParamsError::Encode)?;
    serde_json::from_value(encoded).map_err(ConverseParamsError::Encode)
}

// B2202's detection over the parameters mapped so far: true, type "enabled", or a non-null effort.
pub fn thinking_request(params: &ParamMap) -> Result<ThinkingRequest, ConverseParamsError> {
    Ok(base_thinking_request(&transcode(params)?))
}

// Python truthiness: null, false, 0, "" and empty collections are falsy.
#[must_use]
pub fn truthy(value: &ParamValue) -> bool {
    match value {
        ParamValue::Null => false,
        ParamValue::Bool(flag) => *flag,
        ParamValue::Number(number) => number.as_f64().is_none_or(|parsed| parsed != 0.0_f64),
        ParamValue::Text(text) => !text.is_empty(),
        ParamValue::List(items) => !items.is_empty(),
        ParamValue::Mapping(fields) => !fields.is_empty(),
    }
}

// Python's int(value) over the shapes a request carries: booleans and int-shaped numbers.
#[must_use]
pub fn python_int(value: &ParamValue) -> Option<i64> {
    match value {
        ParamValue::Bool(flag) => Some(i64::from(*flag)),
        ParamValue::Number(number) => number.as_i64(),
        ParamValue::Null | ParamValue::Text(_) | ParamValue::List(_) | ParamValue::Mapping(_) => {
            None
        }
    }
}
