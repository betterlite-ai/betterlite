use betterlite_openai::batches::sdk::{Timeout, TimeoutPhases};

use super::error::BatchProviderListError;
use super::{BatchListTimeoutValue, BatchProviderListInput};

pub(super) const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;

pub(super) fn resolve_timeout(
    input: &BatchProviderListInput,
    supports_phases: bool,
) -> Result<Timeout, BatchProviderListError> {
    let Some(selected) = input.timeout.as_ref() else {
        return finite_timeout(input.request_timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS));
    };
    match selected {
        BatchListTimeoutValue::Phases(phases) if !supports_phases => {
            finite_timeout(phases.read.unwrap_or(DEFAULT_TIMEOUT_SECONDS))
        }
        BatchListTimeoutValue::Phases(phases) => {
            validate_phases(phases).map(|()| Timeout::Phases(phases.clone()))
        }
        BatchListTimeoutValue::Seconds(seconds) => finite_timeout(*seconds),
        BatchListTimeoutValue::Text(text) => parse_timeout(text).and_then(finite_timeout),
    }
}

const fn finite_timeout(seconds: f64) -> Result<Timeout, BatchProviderListError> {
    if seconds.is_finite() {
        Ok(Timeout::Seconds(seconds))
    } else {
        Err(BatchProviderListError::NonFiniteTimeout)
    }
}

fn parse_timeout(text: &str) -> Result<f64, BatchProviderListError> {
    text.trim()
        .parse::<f64>()
        .map_err(|source| BatchProviderListError::InvalidTimeout { source })
}

fn validate_phases(phases: &TimeoutPhases) -> Result<(), BatchProviderListError> {
    let values = [phases.connect, phases.read, phases.write, phases.pool];
    if values.into_iter().flatten().all(f64::is_finite) {
        Ok(())
    } else {
        Err(BatchProviderListError::NonFiniteTimeout)
    }
}
