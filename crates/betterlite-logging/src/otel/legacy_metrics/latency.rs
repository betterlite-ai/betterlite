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

fn elapsed(from: Decimal, to: Decimal) -> Result<Decimal, LegacyMetricsError> {
    to.checked_sub(from).ok_or(LegacyMetricsError::TimeRange)
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
    let moments = (
        stamp(context.kwargs, "api_call_start_time", context.resolver)?,
        stamp(context.kwargs, "completion_start_time", context.resolver)?,
    );
    if let (true, (Stamp::At(call_start), Stamp::At(first_token))) = (streaming, moments) {
        record_seconds(
            histogram,
            elapsed(call_start, first_token)?,
            context.attributes,
        );
    }
    Ok(())
}

pub fn record_tpot_metric(
    histogram: &Histogram<f64>,
    context: &LatencyContext<'_>,
    completion_tokens: Decimal,
) -> Result<(), LegacyMetricsError> {
    let generation = match (
        stamp(context.kwargs, "completion_start_time", context.resolver)?,
        stamp(context.kwargs, "api_call_start_time", context.resolver)?,
    ) {
        (Stamp::At(started), _) | (Stamp::Absent, Stamp::At(started)) => {
            elapsed(started, context.end_time)?
        }
        (Stamp::Unparsed, _) | (Stamp::Absent, Stamp::Unparsed | Stamp::Absent) => context.duration,
    };
    let per_token = generation
        .checked_div(completion_tokens)
        .ok_or(LegacyMetricsError::TimeRange)?;
    if generation > Decimal::ZERO {
        record_seconds(histogram, per_token, context.attributes);
    }
    Ok(())
}

pub fn record_api_latency_metric(
    histogram: &Histogram<f64>,
    context: &LatencyContext<'_>,
) -> Result<(), LegacyMetricsError> {
    let span = match (
        stamp(context.kwargs, "api_call_start_time", context.resolver)?,
        stamp(context.kwargs, "end_time", context.resolver)?,
    ) {
        (Stamp::At(call_start), Stamp::Absent) => elapsed(call_start, context.end_time)?,
        (Stamp::At(call_start), Stamp::At(finished)) => elapsed(call_start, finished)?,
        (Stamp::Absent | Stamp::Unparsed, _) | (Stamp::At(_), Stamp::Unparsed) => return Ok(()),
    };
    if span > Decimal::ZERO {
        record_seconds(histogram, span, context.attributes);
    }
    Ok(())
}
