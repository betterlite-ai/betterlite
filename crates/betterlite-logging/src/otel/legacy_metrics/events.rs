use opentelemetry::logs::{AnyValue, LogRecord as _, Logger as _, Severity};
use opentelemetry::trace::SpanContext;
use opentelemetry_sdk::logs::SdkLogger;

use super::attributes::provider_label;
use super::error::LegacyMetricsError;
use super::inference_details::emit_inference_details;
use super::legacy_content_events::emit_content_events;
use crate::otel::legacy_policy::capture::CaptureMode;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::events::Clock;

pub type EventAttributes = Vec<(&'static str, AnyValue)>;

pub struct SemanticEventEmitter {
    pub event_logger: SdkLogger,
    pub clock: Box<dyn Clock>,
    pub enable_events: bool,
    pub semconv_latest: bool,
}

pub struct EventCall<'call> {
    pub kwargs: &'call MetadataMap,
    pub response: Option<&'call OtelModelValue>,
    pub span_context: &'call SpanContext,
    pub capture_mode: CaptureMode,
}

impl SemanticEventEmitter {
    pub fn emit_semantic_logs(&self, call: &EventCall<'_>) -> Result<(), LegacyMetricsError> {
        if !self.enable_events {
            return Ok(());
        }
        let provider = provider_label(call.kwargs);
        if self.semconv_latest {
            return emit_inference_details(self, call, provider);
        }
        emit_content_events(self, call, provider)
    }

    // every semantic event is an INFO record bound to the inference span by its trace context
    pub fn emit_record(
        &self,
        span_context: &SpanContext,
        body: Option<AnyValue>,
        attributes: EventAttributes,
    ) {
        let mut record = self.event_logger.create_log_record();
        record.set_timestamp(self.clock.now());
        record.set_trace_context(
            span_context.trace_id(),
            span_context.span_id(),
            Some(span_context.trace_flags()),
        );
        record.set_severity_number(Severity::Info);
        record.set_severity_text("INFO");
        if let Some(content) = body {
            record.set_body(content);
        }
        record.add_attributes(attributes);
        self.event_logger.emit(record);
    }
}

#[must_use]
pub fn choices_of(response: Option<&OtelModelValue>) -> &[OtelModelValue] {
    response
        .and_then(OtelModelValue::as_object)
        .and_then(|fields| fields.get("choices"))
        .and_then(OtelModelValue::as_array)
        .unwrap_or_default()
}

pub(super) mod tests;
