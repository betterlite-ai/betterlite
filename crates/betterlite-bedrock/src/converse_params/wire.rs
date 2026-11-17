use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::converse_params::error::ConverseParamsError;
use crate::model_routes::ParamValue;

// The prerequisites read their own JSON records, so a typed value crosses that boundary and comes back.
pub fn transcode<T, U>(value: &U) -> Result<T, ConverseParamsError>
where
    T: DeserializeOwned,
    U: Serialize + ?Sized,
{
    serde_json::from_value(serde_json::to_value(value).map_err(ConverseParamsError::Encode)?)
        .map_err(ConverseParamsError::Encode)
}

// Python's `or` and `not` read null, false, 0, "" and empty collections as no value.
#[must_use]
pub fn carries_value(value: &ParamValue) -> bool {
    match value {
        ParamValue::Null => false,
        ParamValue::Bool(flag) => *flag,
        ParamValue::Number(number) => number.as_f64().is_none_or(|parsed| parsed != 0.0_f64),
        ParamValue::Text(text) => !text.is_empty(),
        ParamValue::List(items) => !items.is_empty(),
        ParamValue::Mapping(fields) => !fields.is_empty(),
    }
}
