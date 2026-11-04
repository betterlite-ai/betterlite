#![cfg(test)]

use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use rstest::rstest;

use super::{BilledAt, coerce_billed_at, decode_billed_at};

fn assert_decodes_to(text: &str, expected: Option<&str>) {
    let decoded = decode_billed_at(text);
    let expected_instant = expected.map(|value| {
        DateTime::parse_from_rfc3339(value)
            .expect("the expected fixture is itself a valid offset timestamp")
            .with_timezone(&Utc)
    });
    assert_eq!(decoded, expected_instant);
}

#[rstest]
#[case::zulu_offset("2024-03-01T10:00:00Z", Some("2024-03-01T10:00:00+00:00"))]
#[case::explicit_offset("2024-03-01T10:00:00+02:00", Some("2024-03-01T08:00:00+00:00"))]
#[case::fractional_seconds("2024-03-01T10:00:00.5Z", Some("2024-03-01T10:00:00.500+00:00"))]
#[case::empty("", None)]
#[case::not_a_date("not-a-date", None)]
#[case::naive_without_offset("2024-03-01T10:00:00", Some("2024-03-01T10:00:00+00:00"))]
#[case::date_only("2024-03-01", Some("2024-03-01T00:00:00+00:00"))]
#[case::offset_without_colon("2024-03-01T10:00:00+0200", Some("2024-03-01T08:00:00+00:00"))]
#[case::basic_format("20240301T100000", Some("2024-03-01T10:00:00+00:00"))]
#[case::doubled_marker("2024-03-01T10:00:00ZZ", None)]
#[case::space_separator("2024-03-01 10:00:00+00:00", Some("2024-03-01T10:00:00+00:00"))]
#[case::hour_and_minute("2024-03-01T10:00", Some("2024-03-01T10:00:00+00:00"))]
#[case::hour_only("2024-03-01T10", Some("2024-03-01T10:00:00+00:00"))]
#[case::hour_and_minute_with_offset("2024-03-01T10:00+02:00", Some("2024-03-01T08:00:00+00:00"))]
fn decode_billed_at_matches_coerced_iso_offsets(
    #[case] text: &str,
    #[case] expected: Option<&str>,
) {
    assert_decodes_to(text, expected);
}

#[test]
fn coerce_billed_at_passes_a_typed_timestamp_through_unchanged() {
    let timestamp: DateTime<Utc> = "2024-03-01T10:00:00Z"
        .parse()
        .expect("a fixture timestamp parses");
    assert_eq!(
        coerce_billed_at(BilledAt::Timestamp(timestamp)),
        Some(timestamp)
    );
}

#[test]
fn coerce_billed_at_returns_none_for_a_missing_value() {
    assert_eq!(coerce_billed_at(BilledAt::Missing), None);
}

#[test]
fn coerce_billed_at_decodes_a_text_value() {
    assert!(coerce_billed_at(BilledAt::Text("2024-03-01T10:00:00Z".to_owned())).is_some());
}

#[rstest]
#[case::comma_fraction("2024-03-01T10:00:00,5Z", Some("2024-03-01T10:00:00.500+00:00"))]
#[case::week_date("2024-W09-5T10:00:00Z", Some("2024-03-01T10:00:00+00:00"))]
#[case::week_date_basic("2024W095T10:00:00Z", Some("2024-03-01T10:00:00+00:00"))]
#[case::week_date_only("2024-W09", Some("2024-02-26T00:00:00+00:00"))]
#[case::hour_only_offset("2024-03-01T10:00:00+02", Some("2024-03-01T08:00:00+00:00"))]
#[case::offset_with_seconds("2024-03-01T10:00:00+02:00:30", Some("2024-03-01T07:59:30+00:00"))]
#[case::basic_time("2024-03-01T100000Z", Some("2024-03-01T10:00:00+00:00"))]
#[case::basic_date("20240301T10:00:00Z", Some("2024-03-01T10:00:00+00:00"))]
#[case::date_time_marker_only("2024-03-01T", Some("2024-03-01T00:00:00+00:00"))]
#[case::leap_second_reads_null("2024-03-01T23:59:60Z", None)]
#[case::one_digit_fields_read_null("2024-3-1", None)]
#[case::year_zero("0000-01-01", None)]
#[case::offset_at_24_hours("2024-03-01T10:00:00+24:00", None)]
#[case::inconsistent_time_separator("2024-03-01T10:0000", None)]
#[case::microseconds_truncate_past_six_digits(
    "2024-03-01T10:00:00.1234567Z",
    Some("2024-03-01T10:00:00.123456+00:00")
)]
#[case::multi_byte_separator("2024-03-01\u{e9}10:00:00Z", Some("2024-03-01T10:00:00+00:00"))]
fn decode_billed_at_follows_the_fromisoformat_grammar(
    #[case] text: &str,
    #[case] expected: Option<&str>,
) {
    assert_decodes_to(text, expected);
}

#[rstest]
fn replays_the_billed_at_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/billed_at");
    for entry in fs::read_dir(directory).expect("the billed_at corpus directory exists") {
        let path = entry.expect("a corpus entry is readable").path();
        if let Ok(text) = fs::read_to_string(path) {
            let _decoded: Option<DateTime<Utc>> = decode_billed_at(&text);
        }
    }
}
