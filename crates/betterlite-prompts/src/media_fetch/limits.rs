use std::time::Duration;

use betterlite_context::core_settings::CoreSettings;
use betterlite_http::proxy::limits::TransportLimits;

// Duration::MAX overflows into tokio's far-future deadline, which is LiteLLM's no-timeout (aiohttp reads 0 as no timeout)
const LONGEST_TIMEOUT: Duration = Duration::MAX;

// LiteLLM's module-level clients take litellm_settings.request_timeout, else REQUEST_TIMEOUT, else 6000 seconds for connect and each read
#[must_use]
pub fn transport_limits(core: &CoreSettings) -> TransportLimits {
    let seconds = core
        .litellm_request_timeout
        .unwrap_or(core.request_timeout_seconds);
    let timeout = if seconds == 0.0_f64 {
        LONGEST_TIMEOUT
    } else if !seconds.is_finite() || seconds.is_sign_negative() {
        // aiohttp rejects a negative or infinite timeout on every request, so each try fails
        Duration::ZERO
    } else {
        match Duration::try_from_secs_f64(seconds) {
            Ok(timeout) => timeout,
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
