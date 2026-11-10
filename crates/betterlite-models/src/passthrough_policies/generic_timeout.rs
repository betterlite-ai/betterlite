pub const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;

#[must_use]
pub const fn resolve_passthrough_timeout(
    endpoint_timeout: Option<f64>,
    settings_timeout: f64,
) -> f64 {
    let Some(seconds) = endpoint_timeout else {
        return settings_timeout;
    };
    seconds
}

mod tests;
