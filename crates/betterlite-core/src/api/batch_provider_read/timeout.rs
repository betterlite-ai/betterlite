use betterlite_openai::batches::sdk::{Timeout, TimeoutPhases};

use super::{BatchProviderReadInput, BatchReadTimeoutValue};
use crate::api::resource_sdk_settings::timeout::TimeoutError;

pub(super) const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;

// optional_params.timeout wins, then request_timeout, then 600 seconds; only openai, azure and bedrock keep the pieces
pub(super) fn resolve_timeout(input: &BatchProviderReadInput) -> Result<Timeout, TimeoutError> {
    let timeout = match input.optional_params.timeout.as_ref() {
        None => Timeout::Seconds(finite(
            input
                .optional_params
                .request_timeout
                .unwrap_or(DEFAULT_TIMEOUT_SECONDS),
        )?),
        Some(BatchReadTimeoutValue::Seconds(seconds)) => Timeout::Seconds(finite(*seconds)?),
        Some(BatchReadTimeoutValue::Text(text)) => Timeout::Seconds(finite(parse_text(text)?)?),
        Some(BatchReadTimeoutValue::Phases(phases)) if input.provider.supports_httpx_timeout() => {
            check_phases(phases)?;
            Timeout::Phases(phases.clone())
        }
        Some(BatchReadTimeoutValue::Phases(phases)) => {
            Timeout::Seconds(finite(phases.read.unwrap_or(DEFAULT_TIMEOUT_SECONDS))?)
        }
    };
    Ok(timeout)
}

fn parse_text(text: &str) -> Result<f64, TimeoutError> {
    text.trim()
        .parse::<f64>()
        .map_err(|source| TimeoutError::InvalidNumber { source })
}

// JSON has no representation for the infinities a float field can still carry
const fn finite(seconds: f64) -> Result<f64, TimeoutError> {
    if seconds.is_finite() {
        Ok(seconds)
    } else {
        Err(TimeoutError::NonFinite)
    }
}

fn check_phases(phases: &TimeoutPhases) -> Result<(), TimeoutError> {
    let values = [phases.connect, phases.read, phases.write, phases.pool];
    if values.into_iter().flatten().all(f64::is_finite) {
        Ok(())
    } else {
        Err(TimeoutError::NonFinite)
    }
}
