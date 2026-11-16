#![cfg(test)]

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opentelemetry::logs::{AnyValue, LoggerProvider as _, Severity};
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLogRecord, SdkLoggerProvider};
use rstest::rstest;

use super::{EventCall, SemanticEventEmitter};
use crate::otel::legacy_metrics::error::LegacyMetricsError;
use crate::otel::legacy_policy::capture::CaptureMode;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::events::Clock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Disabled,
    Legacy,
    Details,
}

struct FixedClock;

fn fixed_time() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .expect("time in range")
}

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        fixed_time()
    }
}

pub(crate) fn span_context() -> SpanContext {
    SpanContext::new(
        TraceId::from(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10_u128),
        SpanId::from(0x1112_1314_1516_1718_u64),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    )
}

pub(crate) fn try_emit(
    mode: Mode,
    capture_mode: CaptureMode,
    kwargs_text: &str,
    response_text: Option<&str>,
) -> Result<Vec<SdkLogRecord>, LegacyMetricsError> {
    let exporter = InMemoryLogExporter::default();
    let provider = SdkLoggerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let emitter = SemanticEventEmitter {
        event_logger: provider.logger("legacy-events-test"),
        clock: Box::new(FixedClock),
        enable_events: mode != Mode::Disabled,
        semconv_latest: mode == Mode::Details,
    };
    let kwargs: MetadataMap = serde_json::from_str(kwargs_text).expect("kwargs");
    let response: Option<OtelModelValue> =
        response_text.map(|text| serde_json::from_str(text).expect("response"));
    let context = span_context();
    emitter.emit_semantic_logs(&EventCall {
        kwargs: &kwargs,
        response: response.as_ref(),
        span_context: &context,
        capture_mode,
    })?;
    let emitted = exporter.get_emitted_logs().expect("emitted logs");
    Ok(emitted.into_iter().map(|data| data.record).collect())
}

pub(crate) fn attributes_of(record: &SdkLogRecord) -> BTreeMap<String, AnyValue> {
    record
        .attributes_iter()
        .map(|(key, value)| (key.to_string(), value.clone()))
        .collect()
}

pub(crate) fn expected_attributes(pairs: &[(&str, AnyValue)]) -> BTreeMap<String, AnyValue> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

const CALL: &str = r#"{
    "call_type": "acompletion",
    "litellm_params": {"custom_llm_provider": "openai"},
    "messages": [{"role": "user", "content": "hello"}]
}"#;
const REPLY: &str = r#"{"choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": "hi"}}]}"#;

#[rstest]
#[case::legacy(Mode::Legacy, 2)]
#[case::details(Mode::Details, 1)]
#[case::disabled(Mode::Disabled, 0)]
fn events_follow_the_enable_flag_and_the_semconv_opt_in(
    #[case] mode: Mode,
    #[case] emitted: usize,
) {
    let records = try_emit(mode, CaptureMode::SpanAndEvent, CALL, Some(REPLY)).expect("emit");
    assert_eq!(records.len(), emitted);
}

#[test]
fn every_event_is_an_info_record_bound_to_the_inference_span() {
    let records =
        try_emit(Mode::Legacy, CaptureMode::SpanAndEvent, CALL, Some(REPLY)).expect("emit");
    let context = span_context();
    let expected = (
        Some(Severity::Info),
        Some("INFO"),
        Some(fixed_time()),
        Some((
            context.trace_id(),
            context.span_id(),
            Some(context.trace_flags()),
        )),
    );
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| {
        let trace = record
            .trace_context()
            .map(|found| (found.trace_id, found.span_id, found.trace_flags));
        (
            record.severity_number(),
            record.severity_text(),
            record.timestamp(),
            trace,
        ) == expected
    }));
}

#[test]
fn a_disabled_emitter_ignores_even_a_malformed_call() {
    let records = try_emit(
        Mode::Disabled,
        CaptureMode::SpanAndEvent,
        r#"{"messages": ["text"]}"#,
        None,
    );
    assert!(records.expect("nothing is read").is_empty());
}

#[test]
fn a_call_without_messages_or_choices_emits_nothing_in_the_legacy_path() {
    let records = try_emit(Mode::Legacy, CaptureMode::SpanAndEvent, "{}", None).expect("emit");
    assert!(records.is_empty());
}
