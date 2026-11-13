use super::handler::SuccessHandler;
use crate::callback_logger::error::LoggerError;
use crate::exporter_options::failure_counter::{
    CallbackFailureCounter, handle_callback_failure as count_callback_failure,
};

// LiteLLM's _handle_callback_failure: a warning and the first registered counter, never an error out
pub fn handle_callback_failure(
    handler: &SuccessHandler<'_>,
    callback_name: &str,
    error: &LoggerError,
) {
    tracing::warn!(
        callback = callback_name,
        %error,
        "LiteLLM.LoggingError: [Non-Blocking] success logging failed"
    );
    let mut counters: Vec<&dyn CallbackFailureCounter> =
        Vec::with_capacity(handler.failure_counters.len());
    for counter in handler.failure_counters {
        counters.push(counter.as_ref());
    }
    count_callback_failure(callback_name, &counters);
}
