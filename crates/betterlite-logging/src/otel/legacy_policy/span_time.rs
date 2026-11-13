use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::otel::legacy_policy::text_field;
use crate::otel::model::utils::coercion::OtelModelValue;

mod tests;

pub const LITELLM_REQUEST_SPAN_NAME: &str = "litellm_request";
pub const UNKNOWN_MODEL: &str = "unknown";
const NANOS_PER_SECOND: f64 = 1_000_000_000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpanTime {
    Missing,
    Seconds(f64),
    DateTime(DateTime<Utc>),
}

#[must_use]
pub fn to_ns(time: SpanTime, now: i64) -> i64 {
    let nanoseconds = match time {
        SpanTime::Missing => return now,
        SpanTime::Seconds(seconds) => seconds.mul_add(NANOS_PER_SECOND, 0.0).trunc(),
        SpanTime::DateTime(datetime) => datetime_seconds(datetime)
            .mul_add(NANOS_PER_SECOND, 0.0)
            .trunc(),
    };
    let converted = Decimal::from_f64_retain(nanoseconds).and_then(|decimal| decimal.to_i64());
    converted.unwrap_or_else(|| {
        if nanoseconds.is_sign_negative() {
            i64::MIN
        } else {
            i64::MAX
        }
    })
}

#[must_use]
pub fn span_name(
    litellm_params: &BTreeMap<String, OtelModelValue>,
    operation: &str,
    model: Option<&str>,
    semconv_latest_experimental: bool,
) -> String {
    let generation_name = litellm_params
        .get("metadata")
        .and_then(OtelModelValue::as_object)
        .and_then(|metadata| text_field(metadata, "generation_name"));
    if let Some(name) = generation_name {
        return name.to_owned();
    }
    if semconv_latest_experimental {
        let resolved = model
            .filter(|text| !text.is_empty())
            .unwrap_or(UNKNOWN_MODEL);
        return format!("{operation} {resolved}");
    }
    LITELLM_REQUEST_SPAN_NAME.to_owned()
}

fn datetime_seconds(datetime: DateTime<Utc>) -> f64 {
    let seconds = Decimal::from(datetime.timestamp());
    let nanos = Decimal::new(i64::from(datetime.timestamp_subsec_nanos()), 9);
    seconds
        .checked_add(nanos)
        .and_then(|total| total.to_f64())
        .unwrap_or_default()
}
