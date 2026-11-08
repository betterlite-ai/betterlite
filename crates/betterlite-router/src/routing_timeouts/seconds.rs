use std::time::Duration;

use super::error::TimeoutError;

pub const DEFAULT_REQUEST_TIMEOUT: Seconds = Seconds(6000.0);
pub const COMPLETION_FALLBACK: Seconds = Seconds(600.0);

// constructed through Seconds::new or the two pinned constants, so a stored value is finite and non-negative
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seconds(pub f64);

impl Seconds {
    pub fn new(field: &str, value: f64) -> Result<Self, TimeoutError> {
        if !value.is_finite() {
            return Err(TimeoutError::NotFinite {
                field: field.to_owned(),
                seconds: value,
            });
        }
        if value < 0.0 {
            return Err(TimeoutError::Negative {
                field: field.to_owned(),
                seconds: value,
            });
        }
        Ok(Self(value))
    }

    pub fn duration(self, field: &str) -> Result<Duration, TimeoutError> {
        Duration::try_from_secs_f64(self.0).map_err(|_source| TimeoutError::OutOfRange {
            field: field.to_owned(),
            seconds: self.0,
        })
    }
}
