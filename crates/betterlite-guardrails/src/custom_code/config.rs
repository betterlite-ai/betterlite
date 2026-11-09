use std::time::Duration;

use super::error::CustomCodeGuardrailError;

pub const DEFAULT_TIMEOUT_SECONDS: f64 = 30.0;
pub const DEFAULT_GUARDRAIL_NAME: &str = "custom_code";

#[derive(Debug, Clone, Default)]
pub struct CustomCodeGuardrailParams {
    pub custom_code: Option<String>,
    pub guardrail_name: Option<String>,
    pub timeout: Option<f64>,
}

// LiteLLM requires custom_code and an optional timeout above zero; both checks live here.
pub fn resolve_timeout(timeout: Option<f64>) -> Result<(f64, Duration), CustomCodeGuardrailError> {
    let seconds = timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    if seconds <= 0.0_f64 || !seconds.is_finite() {
        return Err(CustomCodeGuardrailError::NonPositiveTimeout { timeout: seconds });
    }
    let budget = Duration::try_from_secs_f64(seconds).map_err(|source| {
        CustomCodeGuardrailError::TimeoutOutOfRange {
            timeout: seconds,
            source,
        }
    })?;
    Ok((seconds, budget))
}
