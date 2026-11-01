#![cfg(test)]

use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use rstest::rstest;

use super::{BilledAt, coerce_billed_at, decode_billed_at};

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
    let decoded = decode_billed_at(text);
    let expected_instant = expected.map(|value| {
        DateTime::parse_from_rfc3339(value)
            .expect("the expected fixture is itself a valid offset timestamp")
            .with_timezone(&Utc)
    });
    assert_eq!(decoded, expected_instant);
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
fn replays_the_billed_at_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/billed_at");
    for entry in fs::read_dir(directory).expect("the billed_at corpus directory exists") {
        let path = entry.expect("a corpus entry is readable").path();
        if let Ok(text) = fs::read_to_string(path) {
            let _decoded: Option<DateTime<Utc>> = decode_billed_at(&text);
        }
    }
}
