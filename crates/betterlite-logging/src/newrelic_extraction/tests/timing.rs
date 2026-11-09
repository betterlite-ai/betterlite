use chrono::{DateTime, FixedOffset};
use rstest::rstest;

use super::fixtures::{call_kwargs, object, payload, sources};
use crate::fields::timestamps::Moment;
use crate::newrelic_extraction::ExtractionError;
use crate::newrelic_extraction::timing::{duration, milliseconds_as_int, to_epoch_ms};

fn decimal_seconds(text: &str) -> Moment {
    Moment::Float(text.parse().unwrap())
}

fn datetime(text: &str) -> Moment {
    let parsed: DateTime<FixedOffset> = DateTime::parse_from_rfc3339(text).unwrap();
    Moment::DateTime(parsed)
}

#[rstest]
#[case::float_whole(decimal_seconds("1704067200.0"), 1_704_067_200_000.0_f64)]
#[case::float_fraction(decimal_seconds("1704067200.5"), 1_704_067_200_500.0_f64)]
#[case::integer(Moment::Integer(1_704_067_200), 1_704_067_200_000.0_f64)]
#[case::datetime(datetime("2024-01-01T00:00:00.500+00:00"), 1_704_067_200_500.0_f64)]
fn newrelic_extraction_tests_behavior_epoch_ms(#[case] moment: Moment, #[case] expected: f64) {
    assert_eq!(to_epoch_ms(&moment).unwrap().to_bits(), expected.to_bits());
}

#[rstest]
#[case::whole(1_704_067_200_123.0_f64, 1_704_067_200_123_i64)]
#[case::fraction(1_704_067_200_123.9_f64, 1_704_067_200_123_i64)]
#[case::negative_fraction(-1.9_f64, -1_i64)]
#[case::zero(0.0_f64, 0_i64)]
fn newrelic_extraction_tests_behavior_milliseconds_as_int(
    #[case] milliseconds: f64,
    #[case] expected: i64,
) {
    assert_eq!(milliseconds_as_int(milliseconds).unwrap(), expected);
}

#[test]
fn newrelic_extraction_tests_behavior_milliseconds_as_int_rejects_out_of_range() {
    assert!(matches!(
        milliseconds_as_int(1e30_f64),
        Err(ExtractionError::TimestampRange)
    ));
}

#[test]
fn newrelic_extraction_tests_behavior_duration_prefers_the_payload() {
    let mut standard = payload();
    standard.response_time = "1.5".parse().unwrap();
    let body = object("{}");
    let mut kwargs = call_kwargs();
    kwargs.llm_api_duration_ms = Some(42.0_f64);
    let event_sources = sources(&kwargs, &body, Some(&standard));
    let start = Moment::Integer(1_704_067_200);
    let end = Moment::Integer(1_704_067_203);
    assert_eq!(
        duration(&event_sources, Some(&start), Some(&end))
            .unwrap()
            .map(f64::to_bits),
        Some(1500.0_f64.to_bits())
    );
}

#[test]
fn newrelic_extraction_tests_behavior_duration_keeps_a_zero_response_time() {
    let mut standard = payload();
    standard.response_time = "0".parse().unwrap();
    let body = object("{}");
    let mut kwargs = call_kwargs();
    kwargs.llm_api_duration_ms = Some(42.0_f64);
    let event_sources = sources(&kwargs, &body, Some(&standard));
    assert_eq!(
        duration(&event_sources, None, None)
            .unwrap()
            .map(f64::to_bits),
        Some(0.0_f64.to_bits())
    );
}

#[test]
fn newrelic_extraction_tests_behavior_duration_reads_llm_api_duration() {
    let body = object("{}");
    let mut kwargs = call_kwargs();
    kwargs.llm_api_duration_ms = Some(42.5_f64);
    let event_sources = sources(&kwargs, &body, None);
    let start = Moment::Integer(1_704_067_200);
    let end = Moment::Integer(1_704_067_202);
    assert_eq!(
        duration(&event_sources, Some(&start), Some(&end))
            .unwrap()
            .map(f64::to_bits),
        Some(42.5_f64.to_bits())
    );
}

#[test]
fn newrelic_extraction_tests_behavior_duration_is_calculated_from_times() {
    let body = object("{}");
    let kwargs = call_kwargs();
    let event_sources = sources(&kwargs, &body, None);
    let start = Moment::Integer(1_704_067_200);
    let end = Moment::Integer(1_704_067_202);
    assert_eq!(
        duration(&event_sources, Some(&start), Some(&end))
            .unwrap()
            .map(f64::to_bits),
        Some(2000.0_f64.to_bits())
    );
}

#[test]
fn newrelic_extraction_tests_behavior_duration_needs_both_times() {
    let body = object("{}");
    let kwargs = call_kwargs();
    let event_sources = sources(&kwargs, &body, None);
    let start = Moment::Integer(1_704_067_200);
    assert!(
        duration(&event_sources, Some(&start), None)
            .unwrap()
            .is_none()
    );
    assert!(
        duration(&event_sources, None, Some(&start))
            .unwrap()
            .is_none()
    );
    assert!(duration(&event_sources, None, None).unwrap().is_none());
}
