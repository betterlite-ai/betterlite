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
    let counters: Vec<&dyn CallbackFailureCounter> =
        handler.registry.failure_counter().into_iter().collect();
    count_callback_failure(callback_name, &counters);
}
