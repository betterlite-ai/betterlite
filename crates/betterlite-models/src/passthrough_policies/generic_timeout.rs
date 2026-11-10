pub const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;

#[must_use]
pub fn resolve_passthrough_timeout(endpoint_timeout: Option<f64>, settings_timeout: f64) -> f64 {
    match endpoint_timeout {
        Some(seconds) => seconds,
        None => settings_timeout,
    }
}

mod tests;
