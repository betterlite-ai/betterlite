use std::ops::{Mul as _, Sub as _};

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::fields::timestamps::Moment;
use crate::newrelic_extraction::ExtractionError;
use crate::newrelic_extraction::sources::EventSources;

const MILLIS_PER_SECOND: f64 = 1000.0;

// LiteLLM multiplies the float seconds by 1000.0, so the decimal text is read back as the double Python held.
pub fn to_epoch_ms(moment: &Moment) -> Result<f64, ExtractionError> {
    let seconds = match moment {
        Moment::DateTime(datetime) => Decimal::new(datetime.timestamp_micros(), 6),
        Moment::Float(value) => *value,
        Moment::Integer(value) => Decimal::from(*value),
    };
    let text = seconds.to_string();
    let parsed = text.parse::<f64>().map_err(ExtractionError::Timestamp)?;
    Ok(parsed.mul(MILLIS_PER_SECOND))
}

// Python's int() truncates the double the same way Decimal's to_i64 does.
pub fn milliseconds_as_int(epoch_ms: f64) -> Result<i64, ExtractionError> {
    let Some(decimal) = Decimal::from_f64_retain(epoch_ms) else {
        return Err(ExtractionError::TimestampRange);
    };
    decimal.to_i64().ok_or(ExtractionError::TimestampRange)
}

pub fn duration(
    sources: &EventSources<'_>,
    start: Option<&Moment>,
    end: Option<&Moment>,
) -> Result<Option<f64>, ExtractionError> {
    if let Some(payload) = sources.standard_logging_object {
        let text = payload.response_time.to_string();
        let seconds = text.parse::<f64>().map_err(ExtractionError::ResponseTime)?;
        return Ok(Some(seconds.mul(MILLIS_PER_SECOND)));
    }
    if let Some(duration_ms) = sources.kwargs.llm_api_duration_ms {
        return Ok(Some(duration_ms));
    }
    match (start, end) {
        (Some(start_value), Some(end_value)) => {
            let elapsed = to_epoch_ms(end_value)?.sub(to_epoch_ms(start_value)?);
            Ok(Some(elapsed))
        }
        (Some(_start_value), None) => Ok(None),
        (None, Some(_end_value)) => Ok(None),
        (None, None) => Ok(None),
    }
}
