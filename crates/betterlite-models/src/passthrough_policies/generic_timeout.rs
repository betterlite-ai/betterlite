pub const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;

// the per-endpoint value wins, then the general setting, then the 600 s default
#[must_use]
pub fn resolve_passthrough_timeout(
    endpoint_timeout: Option<f64>,
    settings_timeout: Option<f64>,
) -> f64 {
    if let Some(seconds) = endpoint_timeout {
        return seconds;
    }
    settings_timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS)
}

mod tests;
