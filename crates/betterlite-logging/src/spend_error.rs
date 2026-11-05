use betterlite_config::collector::TracebackPolicy;
use tokio::sync::watch;

mod tests;

#[must_use]
pub fn should_suppress_tracebacks(
    settings: &watch::Receiver<TracebackPolicy>,
    debug_logging: bool,
) -> bool {
    if debug_logging {
        false
    } else {
        settings.borrow().suppress
    }
}

#[must_use]
pub fn failure_row_traceback(
    settings: &watch::Receiver<TracebackPolicy>,
    traceback: Option<String>,
    debug_logging: bool,
) -> Option<String> {
    if should_suppress_tracebacks(settings, debug_logging) {
        None
    } else {
        traceback
    }
}

pub fn log_spend_error(
    settings: &watch::Receiver<TracebackPolicy>,
    message: &str,
    supplied_trace: Option<&str>,
    active_trace: Option<&str>,
) {
    if should_suppress_tracebacks(settings, tracing::enabled!(tracing::Level::DEBUG)) {
        tracing::error!(message);
    } else if let Some(trace) = supplied_trace.or(active_trace) {
        tracing::error!(message, traceback = trace);
    } else {
        tracing::error!(message);
    }
}
