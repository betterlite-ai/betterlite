use super::common::{field, seconds};
use crate::otel::model::utils::coercion::OtelModelValue;
use rust_decimal::prelude::ToPrimitive as _;

#[must_use]
pub fn time_to_first_chunk_seconds(kwargs: &OtelModelValue) -> Option<f64> {
    let streaming = field(kwargs, "optional_params")
        .and_then(|value| field(value, "stream"))
        .and_then(OtelModelValue::as_bool)
        .unwrap_or(false);
    if !streaming {
        return None;
    }
    let elapsed = seconds(field(kwargs, "completion_start_time"))
        .zip(seconds(field(kwargs, "api_call_start_time")))
        .and_then(|(completion_start, api_call_start)| {
            completion_start.checked_sub(api_call_start)
        })?;
    elapsed.to_f64()
}

mod tests;
