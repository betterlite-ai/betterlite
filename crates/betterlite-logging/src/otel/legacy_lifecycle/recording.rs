use opentelemetry::trace::{Span, SpanContext};
use rust_decimal::Decimal;

use crate::otel::legacy_attributes::context::AttributeContext;
use crate::otel::legacy_attributes::dispatch::set_attributes;
use crate::otel::legacy_metrics::events::EventCall;
use crate::otel::legacy_metrics::recorder::MetricCall;
use crate::otel::legacy_policy::annotations::write_team_attributes;
use crate::otel::legacy_policy::capture::CaptureMode;
use crate::otel::legacy_policy::error_recording::record_exception_on_span;
use crate::otel::legacy_policy::span_time::SpanTime;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::v2_callback::live_span::LiveSpan;

use super::context;
use super::context::standard_logging_object;
use super::error::LegacyLifecycleError;
use super::lifecycle::{LegacyLifecycle, LifecycleCall};

pub(super) fn apply_attributes(
    lifecycle: &LegacyLifecycle,
    span: &LiveSpan,
    call: &LifecycleCall<'_>,
    capture_mode: CaptureMode,
) {
    let attributes = AttributeContext {
        callback_name: lifecycle.callback_name.as_deref(),
        config: &lifecycle.config,
        capture_mode,
    };
    let mut target = span.lock();
    if let Err(error) = set_attributes(&mut *target, &attributes, call.kwargs, call.response) {
        drop(target);
        tracing::warn!(%error, "OpenTelemetry logging error in set_attributes");
        context::count_failure(lifecycle);
    }
}

pub(super) fn record_exception(span: &LiveSpan, call: &LifecycleCall<'_>) {
    let empty = MetadataMap::new();
    let payload = standard_logging_object(call.kwargs).unwrap_or(&empty);
    record_exception_on_span(&mut *span.lock(), call.exception, payload);
}

pub(super) fn write_team_attributes_from_log<S>(span: &mut S, kwargs: &MetadataMap)
where
    S: Span,
{
    if let Some(payload) = standard_logging_object(kwargs) {
        write_team_attributes(span, payload);
    }
}

pub(super) fn record_metrics(
    lifecycle: &LegacyLifecycle,
    call: &LifecycleCall<'_>,
) -> Result<(), LegacyLifecycleError> {
    let Some(recorder) = &lifecycle.metrics else {
        return Ok(());
    };
    recorder.record_metrics(&MetricCall {
        kwargs: call.kwargs,
        response: call.response,
        start_time: metric_seconds(lifecycle, call.start_time),
        end_time: metric_seconds(lifecycle, call.end_time),
    })?;
    Ok(())
}

// the recorder wants epoch seconds; a missing stamp falls back to the clock
pub(super) fn metric_seconds(lifecycle: &LegacyLifecycle, time: SpanTime) -> Decimal {
    match time {
        SpanTime::Missing => Decimal::from(lifecycle.now_ns())
            .checked_div(Decimal::from(1_000_000_000_i64))
            .unwrap_or_default(),
        SpanTime::Seconds(value) => Decimal::from_f64_retain(value).unwrap_or_default(),
        SpanTime::DateTime(datetime) => Decimal::from(datetime.timestamp())
            .checked_add(Decimal::new(
                i64::from(datetime.timestamp_subsec_nanos()),
                9,
            ))
            .unwrap_or_default(),
    }
}

pub(super) fn emit_semantic_logs(
    lifecycle: &LegacyLifecycle,
    call: &LifecycleCall<'_>,
    span_context: &SpanContext,
) -> Result<(), LegacyLifecycleError> {
    let Some(events) = &lifecycle.events else {
        return Ok(());
    };
    events.emit_semantic_logs(&EventCall {
        kwargs: call.kwargs,
        response: call.response,
        span_context,
        capture_mode: lifecycle.capture_policy.resolve(call.flags),
    })?;
    Ok(())
}
