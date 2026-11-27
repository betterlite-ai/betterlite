use std::sync::Arc;

use betterlite_error::proxy_exception::ProxyException;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::otel::logger_runtime::legacy::set_response_status_code;
use betterlite_logging::otel::service::error_attributes::record_error_attributes;
use betterlite_logging::otel::v2_callback::callback::V2Callback;
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;
use betterlite_logging::otel::v2_callback::proxy::record_error_on_span;
use betterlite_logging::otel::v2_callback::span_error::ErrorInformationConfig;
use indexmap::IndexMap;
use opentelemetry::trace::{Span as _, Status};

mod tests;

// the proxy's open_telemetry_logger slot: LiteLLM holds the legacy class or B3853's callback, never both
pub enum ServerSpanLogger {
    // boxed so the variants stay the same size; variant_size_differences denies a 3x gap
    Legacy(Box<ErrorInformationConfig>),
    V2(Arc<V2Callback>),
}

impl ServerSpanLogger {
    // both classes stamp the same error.* attributes, and only the legacy close also ends the span
    fn record_error(&self, live: &LiveSpan, status: u16, error: Option<&ExceptionFacts>) {
        if status < 400 {
            return;
        }
        match self {
            Self::Legacy(config) => {
                record_error_attributes(Some(live), error, i64::from(status), config);
            }
            Self::V2(callback) => record_error_on_span(callback, Some(live), error, status),
        }
    }
}

// litellm.proxy.proxy_server._close_dangling_otel_server_span
pub fn close_error_span(
    span: &mut Option<LiveSpan>,
    logger: Option<&ServerSpanLogger>,
    otel_v2: bool,
    status: u16,
    error: Option<&ExceptionFacts>,
) {
    let Some(live) = span.as_ref() else {
        return;
    };
    let Some(otel_logger) = logger else {
        return;
    };
    // under V2 the FastAPI instrumentor owns the span and ends it, so this only stamps what it would miss
    if otel_v2 {
        otel_logger.record_error(live, status, error);
        return;
    }
    match otel_logger {
        ServerSpanLogger::Legacy(config) => close_legacy_span(live, config, status, error),
        // OpenTelemetryV2 has no set_response_status_code_attribute, so LiteLLM's call raises and its finally clears the slot
        ServerSpanLogger::V2(_) => tracing::debug!(
            "Error closing dangling OTEL SERVER span: 'OpenTelemetryV2' object has no attribute 'set_response_status_code_attribute'"
        ),
    }
    *span = None;
}

fn close_legacy_span(
    live: &LiveSpan,
    config: &ErrorInformationConfig,
    status: u16,
    error: Option<&ExceptionFacts>,
) {
    set_response_status_code(Some(live), Some(i64::from(status)));
    // an ended SDK span ignores later writes, so every attribute goes on before the status and the end
    if status < 400 {
        live.lock().set_status(Status::Ok);
    } else {
        record_error_attributes(Some(live), error, i64::from(status), config);
        live.lock().set_status(Status::error(""));
    }
    live.lock().end();
}

// what litellm's StandardLoggingPayloadSetup.get_error_information reads off a ProxyException
#[must_use]
pub fn exception_facts(class_name: &str, error: &ProxyException) -> ExceptionFacts {
    ExceptionFacts {
        class_name: class_name.to_owned(),
        class: None,
        message: Some(error.message.clone()),
        code: Some(error.code.clone()),
        status_code: None,
        response_status_code: None,
        llm_provider: None,
        category: None,
        rate_limit_type: None,
        budget_entity_type: None,
        budget_entity_id: None,
        budget_limit: None,
        budget_spend: None,
        proxy_type: None,
        headers: Some(
            error
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect::<IndexMap<String, String>>(),
        ),
        response_headers: None,
        litellm_response_headers: None,
        traceback_lines: Vec::new(),
    }
}
