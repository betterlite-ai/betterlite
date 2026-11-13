use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opentelemetry::logs::{AnyValue, LogRecord as _, Logger as _, Severity};
use opentelemetry::trace::SpanContext;
use opentelemetry_sdk::logs::SdkLogger;

use crate::otel::vocabulary::constants::attribute_keys::{exception_event, gen_ai_event};

pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

pub struct OperationException<'span> {
    pub span_context: &'span SpanContext,
    pub error_type: &'span str,
    pub message: &'span str,
    pub stack_trace: Option<&'span str>,
    pub timestamp_ns: Option<u64>,
}

// The GenAI conventions record client exceptions as a WARN log event correlated to the failed span.
pub struct GenAIEventRecorder {
    pub event_logger: SdkLogger,
    pub clock: Box<dyn Clock>,
}

impl GenAIEventRecorder {
    pub fn record_operation_exception(&self, exception: &OperationException<'_>) {
        let mut record = self.event_logger.create_log_record();
        let timestamp = exception
            .timestamp_ns
            .and_then(|nanos| UNIX_EPOCH.checked_add(Duration::from_nanos(nanos)))
            .unwrap_or_else(|| self.clock.now());
        record.set_timestamp(timestamp);
        record.set_trace_context(
            exception.span_context.trace_id(),
            exception.span_context.span_id(),
            Some(exception.span_context.trace_flags()),
        );
        record.set_severity_number(Severity::Warn);
        record.set_body(AnyValue::from(exception.message.to_owned()));
        record.add_attribute(gen_ai_event::NAME_KEY, gen_ai_event::OPERATION_EXCEPTION);
        record.add_attribute(exception_event::TYPE, exception.error_type.to_owned());
        record.add_attribute(exception_event::MESSAGE, exception.message.to_owned());
        if let Some(stack) = exception.stack_trace {
            record.add_attribute(exception_event::STACKTRACE, stack.to_owned());
        }
        self.event_logger.emit(record);
    }
}

#[path = "events/tests.rs"]
mod tests;
