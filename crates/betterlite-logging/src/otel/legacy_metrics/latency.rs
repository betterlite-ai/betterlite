use opentelemetry::KeyValue;
use opentelemetry::metrics::Histogram;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use super::error::LegacyMetricsError;
use crate::otel::integrations::arize_attributes::truthy::is_truthy;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::model::utils::time::{FixedLocalTimeResolver, to_seconds};

mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stamp {
    Absent,
    Unparsed,
    At(Decimal),
}

pub struct LatencyContext<'call> {
    pub kwargs: &'call MetadataMap,
    pub resolver: &'call FixedLocalTimeResolver,
    pub attributes: &'call [KeyValue],
    pub end_time: Decimal,
    pub duration: Decimal,
}

// datetimes reach this crate as epoch seconds, so only numbers and the strptime text forms remain
pub fn stamp(
    kwargs: &MetadataMap,
    field: &'static str,
    resolver: &FixedLocalTimeResolver,
) -> Result<Stamp, LegacyMetricsError> {
    match kwargs.get(field) {
        None | Some(OtelModelValue::Null(())) => Ok(Stamp::Absent),
        Some(number @ OtelModelValue::Number(_)) => {
            Ok(number.as_decimal().map_or(Stamp::Unparsed, Stamp::At))
        }
        Some(OtelModelValue::String(text)) => {
            Ok(to_seconds(None, None, Some(text.as_str()), resolver)
                .map_or(Stamp::Unparsed, Stamp::At))
        }
        Some(OtelModelValue::Bool(_) | OtelModelValue::Array(_) | OtelModelValue::Object(_)) => {
            Err(LegacyMetricsError::WrongType {
                field,
                expected: "a number or a timestamp string",
            })
        }
    }
}

pub fn record_seconds(histogram: &Histogram<f64>, value: Decimal, attributes: &[KeyValue]) {
    if let Some(number) = value.to_f64() {
        histogram.record(number, attributes);
    }
}

fn read_two(
    context: &LatencyContext<'_>,
    first: &'static str,
    second: &'static str,
) -> Result<(Stamp, Stamp), LegacyMetricsError> {
    let first_stamp = stamp(context.kwargs, first, context.resolver)?;
    let second_stamp = stamp(context.kwargs, second, context.resolver)?;
    Ok((first_stamp, second_stamp))
}

pub fn record_ttft_metric(
    histogram: &Histogram<f64>,
    context: &LatencyContext<'_>,
) -> Result<(), LegacyMetricsError> {
    let streaming = context
        .kwargs
        .get("optional_params")
        .and_then(OtelModelValue::as_object)
        .and_then(|params| params.get("stream"))
        .is_some_and(is_truthy);
    if !streaming {
        return Ok(());
    }
    let start = stamp(context.kwargs, "api_call_start_time", context.resolver)?;
    let first = stamp(context.kwargs, "completion_start_time", context.resolver)?;
    let (Stamp::At(call_start), Stamp::At(first_token)) = (start, first) else {
        return Ok(());
    };
    let elapsed = first_token
        .checked_sub(call_start)
        .ok_or(LegacyMetricsError::TimeRange)?;
    record_seconds(histogram, elapsed, context.attributes);
    Ok(())
}

pub fn record_tpot_metric(
    histogram: &Histogram<f64>,
    context: &LatencyContext<'_>,
    completion_tokens: Decimal,
) -> Result<(), LegacyMetricsError> {
    let (completion, start) = read_two(context, "completion_start_time", "api_call_start_time")?;
    let generation = match (completion, start) {
        (Stamp::At(started), _) | (Stamp::Absent, Stamp::At(started)) => context
            .end_time
            .checked_sub(started)
            .ok_or(LegacyMetricsError::TimeRange)?,
        (Stamp::Unparsed, _) | (Stamp::Absent, Stamp::Unparsed | Stamp::Absent) => context.duration,
    };
    if generation <= Decimal::ZERO {
        return Ok(());
    }
    let per_token = generation
        .checked_div(completion_tokens)
        .ok_or(LegacyMetricsError::TimeRange)?;
    record_seconds(histogram, per_token, context.attributes);
    Ok(())
}

pub fn record_api_latency_metric(
    histogram: &Histogram<f64>,
    context: &LatencyContext<'_>,
) -> Result<(), LegacyMetricsError> {
    let (start, end) = read_two(context, "api_call_start_time", "end_time")?;
    let span = match (start, end) {
        (Stamp::At(call_start), Stamp::Absent) => context.end_time.checked_sub(call_start),
        (Stamp::At(call_start), Stamp::At(finished)) => finished.checked_sub(call_start),
        _ => return Ok(()),
    }
    .ok_or(LegacyMetricsError::TimeRange)?;
    if span > Decimal::ZERO {
        record_seconds(histogram, span, context.attributes);
    }
    Ok(())
}
