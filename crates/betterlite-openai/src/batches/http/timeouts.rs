use std::time::Duration;

use crate::batches::error::SdkError;
use crate::batches::sdk::TimeoutPhases;

pub(super) fn duration_seconds(seconds: f64) -> Result<Duration, SdkError> {
    if !seconds.is_finite() || seconds < 0.0_f64 {
        return Err(SdkError::InvalidTimeout { seconds });
    }
    Duration::try_from_secs_f64(seconds).map_err(|_source| SdkError::InvalidTimeout { seconds })
}

// The shared transport applies one total deadline, so every set phase is refused by name
pub(super) fn phase_names(phases: &TimeoutPhases) -> String {
    let mut names = Vec::new();
    if phases.connect.is_some() {
        names.push("connect");
    }
    if phases.read.is_some() {
        names.push("read");
    }
    if phases.write.is_some() {
        names.push("write");
    }
    if phases.pool.is_some() {
        names.push("pool");
    }
    if names.is_empty() {
        String::from("none")
    } else {
        names.join(", ")
    }
}
