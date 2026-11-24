use betterlite_sdk::call_wrapper::error::plain_error;
use thiserror::Error;

use crate::health_probe_modes::error::HealthProbeError;

// a deferred owner answers as not built, so a model the check never called is never reported healthy
#[derive(Debug, Error)]
pub(crate) enum NotBuilt {
    #[error("the {mode} health check is not built")]
    Mode { mode: &'static str },
}

pub(crate) fn not_built(mode: &'static str) -> HealthProbeError {
    let source = NotBuilt::Mode { mode };
    let message = source.to_string();
    HealthProbeError::Backend {
        class: plain_error(&message),
        message,
        source: Box::new(source),
    }
}
