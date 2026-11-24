use std::io;

use betterlite_sdk::call_wrapper::error::plain_error;

use crate::health_probe_modes::error::HealthProbeError;

// a deferred owner answers as not built, so a model the check never called is never reported healthy
pub(crate) fn not_built(mode: &'static str) -> HealthProbeError {
    let message = format!("the {mode} health check is not built");
    let source = io::Error::other(format!("{mode} has no native operation yet"));
    HealthProbeError::Backend {
        class: plain_error(&message),
        message,
        source: Box::new(source),
    }
}
