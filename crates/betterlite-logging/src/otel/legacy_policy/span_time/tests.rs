#![cfg(test)]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rstest::rstest;

use super::{LITELLM_REQUEST_SPAN_NAME, SpanTime, span_name, to_ns};
use crate::otel::model::utils::coercion::OtelModelValue;

fn params(text: &str) -> BTreeMap<String, OtelModelValue> {
    serde_json::from_str(text).expect("litellm params")
}

fn number(text: &str) -> f64 {
    text.parse::<f64>().expect("number")
}

#[test]
fn a_missing_time_uses_the_callers_now() {
    assert_eq!(to_ns(SpanTime::Missing, 1_234_567), 1_234_567);
}

#[rstest]
#[case::whole_second(1.0_f64, 1_000_000_000)]
#[case::half_second(1.5_f64, 1_500_000_000)]
#[case::tenth(number("0.1"), 100_000_000)]
#[case::negative(-1.5_f64, -1_500_000_000)]
#[case::negative_tenth(number("-0.1"), -100_000_000)]
#[case::tiny_positive(number("1e-12"), 0)]
#[case::tiny_negative(number("-1e-12"), 0)]
#[case::zero(0.0_f64, 0)]
fn seconds_truncate_toward_zero_like_int(#[case] seconds: f64, #[case] expected: i64) {
    assert_eq!(to_ns(SpanTime::Seconds(seconds), 0), expected);
}

#[rstest]
#[case::epoch(0, 0, 0)]
#[case::half_second(1, 500_000_000, 1_500_000_000)]
#[case::microsecond(1_700_000_000, 123_456_000, 1_700_000_000_123_456_000)]
fn datetime_times_are_utc_instants(
    #[case] seconds: i64,
    #[case] nanos: u32,
    #[case] expected: i64,
) {
    let datetime = DateTime::<Utc>::from_timestamp(seconds, nanos).expect("datetime");
    assert_eq!(to_ns(SpanTime::DateTime(datetime), 0), expected);
}

#[test]
fn span_name_prefers_the_generation_name() {
    let litellm_params = params(r#"{"metadata": {"generation_name": "custom-name"}}"#);
    assert_eq!(
        span_name(&litellm_params, "chat", Some("gpt-4o"), false),
        "custom-name"
    );
    assert_eq!(
        span_name(&litellm_params, "chat", Some("gpt-4o"), true),
        "custom-name"
    );
}

#[test]
fn an_empty_generation_name_falls_through() {
    let litellm_params = params(r#"{"metadata": {"generation_name": ""}}"#);
    assert_eq!(
        span_name(&litellm_params, "chat", Some("gpt-4o"), true),
        "chat gpt-4o"
    );
}

#[test]
fn semconv_names_the_operation_and_model() {
    let litellm_params = params("{}");
    assert_eq!(
        span_name(&litellm_params, "text_completion", Some("gpt-3.5"), true),
        "text_completion gpt-3.5"
    );
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
fn semconv_defaults_the_model_to_unknown(#[case] model: Option<&str>) {
    let litellm_params = params("{}");
    assert_eq!(
        span_name(&litellm_params, "chat", model, true),
        "chat unknown"
    );
}

#[test]
fn the_legacy_name_is_used_without_semconv() {
    let litellm_params = params("{}");
    assert_eq!(
        span_name(&litellm_params, "chat", Some("gpt-4o"), false),
        LITELLM_REQUEST_SPAN_NAME
    );
}
