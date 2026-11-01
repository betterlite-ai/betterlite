use std::ops::Add as _;

use crate::baseline_history::error::BaselineHistoryError;

pub(crate) const fn ttl_seconds_as_f64(ttl_seconds: i64) -> Result<f64, BaselineHistoryError> {
    match ttl_seconds {
        300 => Ok(300.0),
        3600 => Ok(3600.0),
        _ => Err(BaselineHistoryError::InvalidTtl),
    }
}

pub(crate) fn checked_add_timestamp(base: f64, delta: f64) -> Result<f64, BaselineHistoryError> {
    let sum = base.add(delta);
    if sum.is_finite() {
        Ok(sum)
    } else {
        Err(BaselineHistoryError::TimestampOutOfRange)
    }
}

mod tests;
