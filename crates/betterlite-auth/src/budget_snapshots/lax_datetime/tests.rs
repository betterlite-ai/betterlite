#![cfg(test)]

use super::LaxDateTime;
use rstest::rstest;

fn parse(value: &str) -> LaxDateTime {
    serde_json::from_str(value).unwrap()
}

#[rstest]
#[case::utc(r#""2026-01-01T12:00:00Z""#)]
#[case::offset(r#""2026-01-01T12:00:00+02:00""#)]
#[case::naive_t_separator(r#""2026-01-01T12:00:00""#)]
fn rfc3339_forms_round_trip(#[case] input: &str) {
    let value = parse(input);
    assert_eq!(serde_json::to_string(&value).unwrap(), input);
}

#[test]
fn naive_space_separated_string_is_accepted() {
    let value = parse(r#""2026-01-01 12:00:00""#);
    assert!(matches!(value, LaxDateTime::Naive(_)));
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#""2026-01-01T12:00:00""#
    );
}

#[test]
fn date_only_string_is_midnight_naive() {
    let value = parse(r#""2026-01-01""#);
    assert!(matches!(value, LaxDateTime::Naive(_)));
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#""2026-01-01T00:00:00""#
    );
}

#[test]
fn unix_second_timestamp_is_utc() {
    let value: LaxDateTime = serde_json::from_str("1700000000").unwrap();
    assert!(matches!(value, LaxDateTime::Offset(_)));
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#""2023-11-14T22:13:20Z""#
    );
}

#[test]
fn invalid_string_is_rejected() {
    serde_json::from_str::<LaxDateTime>(r#""not-a-date""#).unwrap_err();
}

#[rstest]
#[case::fractional_seconds_pad_to_six_digits(
    r#""2026-01-01T12:00:00.4Z""#,
    r#""2026-01-01T12:00:00.400000Z""#
)]
#[case::nanosecond_fraction_is_truncated_to_microseconds(
    r#""2026-01-01T12:00:00.123456789Z""#,
    r#""2026-01-01T12:00:00.123456Z""#
)]
fn fractional_second_strings_are_normalized(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(serde_json::to_string(&parse(input)).unwrap(), expected);
}

#[rstest]
#[case::millisecond_timestamp_is_inferred_from_magnitude(
    "1700000000123",
    r#""2023-11-14T22:13:20.123000Z""#
)]
#[case::float_timestamp_seconds_keeps_fraction("1700000000.4", r#""2023-11-14T22:13:20.400000Z""#)]
fn numeric_timestamps_are_utc_offset_with_fraction(#[case] input: &str, #[case] expected: &str) {
    let value: LaxDateTime = serde_json::from_str(input).unwrap();
    assert!(matches!(value, LaxDateTime::Offset(_)));
    assert_eq!(serde_json::to_string(&value).unwrap(), expected);
}
