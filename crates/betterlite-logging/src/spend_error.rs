use std::sync::atomic::{AtomicBool, Ordering};

const SUPPRESS_ENV: &str = "LITELLM_SUPPRESS_SPEND_LOG_TRACEBACKS";
pub static SUPPRESS_TRACEBACKS: AtomicBool = AtomicBool::new(false);

pub fn loads_spend_traceback_policy(value: Option<&str>) -> bool {
    betterlite_env::flag(SUPPRESS_ENV, value.unwrap_or_default())
        .ok()
        .flatten()
        .unwrap_or(false)
}

pub fn should_suppress_tracebacks(debug_logging_enabled: bool) -> bool {
    if debug_logging_enabled {
        return false;
    }
    SUPPRESS_TRACEBACKS.load(Ordering::SeqCst)
}

pub fn failure_row_traceback(
    traceback: Option<String>,
    debug_logging_enabled: bool,
) -> Option<String> {
    if should_suppress_tracebacks(debug_logging_enabled) {
        None
    } else {
        traceback
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpendError {
    #[error("spend error: {0}")]
    Error(String),
}

pub fn log_spend_error(message: &str, trace: Option<&str>, debug_logging_enabled: bool) {
    if should_suppress_tracebacks(debug_logging_enabled) {
        tracing::error!("{message}");
    } else if let Some(trace) = trace {
        tracing::error!("{message}\n{trace}");
    } else {
        tracing::error!("{message}");
    }
}

#[path = "spend_error/tests.rs"]
mod spend_error_tests;
