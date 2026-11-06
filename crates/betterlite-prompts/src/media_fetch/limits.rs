use std::time::{Duration, TryFromFloatSecsError};

use betterlite_context::core_settings::CoreSettings;
use betterlite_http::proxy::limits::TransportLimits;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LimitsError {
    #[error("the request timeout of {seconds} seconds cannot be a duration")]
    Invalid {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
}

// LiteLLM's module-level clients take litellm_settings.request_timeout, else REQUEST_TIMEOUT, else 6000 seconds for connect and each read
pub fn transport_limits(core: &CoreSettings) -> Result<TransportLimits, LimitsError> {
    let seconds = core
        .litellm_request_timeout
        .unwrap_or(core.request_timeout_seconds);
    let timeout = Duration::try_from_secs_f64(seconds)
        .map_err(|source| LimitsError::Invalid { seconds, source })?;
    Ok(TransportLimits {
        connect_timeout: timeout,
        read_timeout: timeout,
        body_bytes: TransportLimits::DEFAULT.body_bytes,
        header_bytes: TransportLimits::DEFAULT.header_bytes,
    })
}
