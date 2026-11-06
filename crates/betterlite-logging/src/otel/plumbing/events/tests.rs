#![cfg(test)]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::otel_settings::Settings as OtelConfig;
use opentelemetry::logs::{AnyValue, Severity};
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLogRecord};
use rstest::rstest;

use super::{Clock, GenAIEventRecorder, OperationException, SystemClock};
use crate::otel::plumbing::providers::{build_logger_provider, get_event_logger};
use crate::otel::plumbing::sinks::LogSink;

fn span_context() -> SpanContext {
    SpanContext::new(
        TraceId::from(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10_u128),
        SpanId::from(0x1112_1314_1516_1718_u64),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    )
}

fn record_exception(stack_trace: Option<&str>, timestamp_ns: Option<u64>) -> SdkLogRecord {
    record_exception_with_clock(Box::new(SystemClock), stack_trace, timestamp_ns)
}

fn record_exception_with_clock(
    clock: Box<dyn Clock>,
    stack_trace: Option<&str>,
    timestamp_ns: Option<u64>,
) -> SdkLogRecord {
    let exporter = InMemoryLogExporter::default();
    let provider =
        build_logger_provider(&OtelConfig::default(), LogSink::InMemory(exporter.clone()));
    let recorder = GenAIEventRecorder {
        event_logger: get_event_logger(&provider, "litellm"),
        clock,
    };
    let context = span_context();
    recorder.record_operation_exception(&OperationException {
        span_context: &context,
        error_type: "RateLimitError",
        message: "slow down",
        stack_trace,
        timestamp_ns,
    });
    let logs = exporter.get_emitted_logs().expect("logs");
    logs.into_iter().next().expect("one log").record
}

fn attributes(record: &SdkLogRecord) -> Vec<(String, String)> {
    record
        .attributes_iter()
        .map(|(key, value)| {
            let text = if let AnyValue::String(text) = value {
                text.to_string()
            } else {
                String::new()
            };
            (key.to_string(), text)
        })
        .collect()
}

#[test]
fn event_operation_exception_is_a_warn_log_with_the_exception_attributes() {
    let record = record_exception(Some("frame one\nframe two"), None);
    assert_eq!(record.severity_number(), Some(Severity::Warn));
    assert_eq!(record.body(), Some(&AnyValue::from("slow down")));
    assert_eq!(
        attributes(&record),
        vec![
            (
                "event.name".to_owned(),
                "gen_ai.client.operation.exception".to_owned()
            ),
            ("exception.type".to_owned(), "RateLimitError".to_owned()),
            ("exception.message".to_owned(), "slow down".to_owned()),
            (
                "exception.stacktrace".to_owned(),
                "frame one\nframe two".to_owned()
            ),
        ]
    );
}

#[rstest]
#[case::absent(None, &["event.name", "exception.type", "exception.message"])]
#[case::empty(
    Some(""),
    &["event.name", "exception.type", "exception.message", "exception.stacktrace"]
)]
fn event_operation_exception_keeps_the_stack_trace_it_is_given(
    #[case] stack: Option<&str>,
    #[case] expected: &[&str],
) {
    let record = record_exception(stack, None);
    let names: Vec<String> = attributes(&record)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, expected);
}

#[test]
fn event_operation_exception_is_correlated_to_the_failed_span() {
    let record = record_exception(None, None);
    let context = record.trace_context().expect("trace context");
    assert_eq!(context.trace_id, span_context().trace_id());
    assert_eq!(context.span_id, span_context().span_id());
    assert_eq!(context.trace_flags, Some(TraceFlags::SAMPLED));
}

#[rstest]
#[case::given(Some(1_700_000_000_123_456_789), Some(1_700_000_000_123_456_789))]
#[case::epoch(Some(0), Some(0))]
fn event_operation_exception_uses_the_given_timestamp(
    #[case] timestamp_ns: Option<u64>,
    #[case] expected: Option<u64>,
) {
    let record = record_exception(None, timestamp_ns);
    let stamped = record.timestamp().expect("timestamp");
    let nanos = stamped
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos());
    assert_eq!(nanos.ok(), expected.map(u128::from));
}

struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

#[test]
fn event_operation_exception_defaults_the_timestamp_to_the_injected_clock() {
    let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let record = record_exception_with_clock(Box::new(FixedClock(now)), None, None);
    assert_eq!(record.timestamp(), Some(now));
}
