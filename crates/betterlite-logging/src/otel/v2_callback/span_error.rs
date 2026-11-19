use std::sync::Arc;

use betterlite_error::normalize::Normalizer;
use betterlite_redact::secret::Redactor;

use crate::logging_metadata::error_info::{ExceptionLogging, get_error_information};
use crate::logging_metadata::exception::ExceptionFacts;
use crate::otel::model::payloads::error::SpanError;
use crate::payload::compose::error_info::expected_client_error;

pub struct ErrorInformationConfig {
    pub normalizer: Normalizer,
    pub redactor: Option<Arc<Redactor>>,
    pub log_client_error_tracebacks: bool,
}

// litellm.integrations.otel.logger._span_error_from_exception
#[must_use]
pub fn span_error_from_exception(
    config: &ErrorInformationConfig,
    exception: Option<&ExceptionFacts>,
    status_code: Option<u16>,
    traceback_str: Option<&str>,
) -> SpanError {
    let info = get_error_information(
        &ExceptionLogging {
            exception,
            log_client_error_tracebacks: config.log_client_error_tracebacks,
            expected_client_error: expected_client_error(exception),
            redactor: config.redactor.as_deref(),
            normalizer: &config.normalizer,
        },
        traceback_str,
    );
    let error_code = text(info.error_code.flatten());
    let code = status_code.map_or_else(|| error_code.clone(), |status| Some(status.to_string()));
    SpanError {
        error_type: text(info.error_class.flatten()).or_else(|| error_code.clone()),
        message: text(info.error_message.flatten()),
        code,
        stack_trace: text(info.traceback.flatten()),
        llm_provider: text(info.llm_provider.flatten()),
    }
}

fn text(value: Option<String>) -> Option<String> {
    match value {
        Some(content) if !content.is_empty() => Some(content),
        Some(_) | None => None,
    }
}
