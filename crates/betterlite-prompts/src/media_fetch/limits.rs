use std::time::Duration;

use betterlite_context::core_settings::CoreSettings;
use betterlite_http::proxy::limits::TransportLimits;

// The longest span an Instant takes without panicking: tokio's own far-future is 30 years, so anything longer is clamped here
const LONGEST_TIMEOUT: Duration = Duration::from_hours(262_800);

// LiteLLM's module-level clients take litellm_settings.request_timeout, else REQUEST_TIMEOUT, else 6000 seconds for connect and each read
#[must_use]
pub fn transport_limits(core: &CoreSettings) -> TransportLimits {
    let seconds = core
        .litellm_request_timeout
        .unwrap_or(core.request_timeout_seconds);
    let timeout = if seconds == 0.0_f64 || seconds.is_nan() {
        // aiohttp reads 0 as no timeout, and a NaN timeout never fires, so the fetch finishes
        LONGEST_TIMEOUT
    } else if !seconds.is_finite() || seconds.is_sign_negative() {
        // aiohttp rejects a negative or infinite timeout on every request, so each try fails
        Duration::ZERO
    } else {
        match Duration::try_from_secs_f64(seconds) {
            Ok(timeout) => timeout.min(LONGEST_TIMEOUT),
            Err(_error) => LONGEST_TIMEOUT,
        }
    };
    TransportLimits {
        connect_timeout: timeout,
        read_timeout: timeout,
        body_bytes: TransportLimits::DEFAULT.body_bytes,
        header_bytes: TransportLimits::DEFAULT.header_bytes,
    }
}
