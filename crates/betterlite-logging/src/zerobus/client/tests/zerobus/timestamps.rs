use rstest::rstest;

use super::support::row;

#[rstest]
#[case::whole_seconds("1700000000", 1_700_000_000_000_000)]
#[case::quarter_second("1700000000.25", 1_700_000_000_250_000)]
#[case::one_microsecond("1700000000.000001", 1_700_000_000_000_001)]
fn positive_epoch_seconds_become_epoch_microseconds(#[case] value: &str, #[case] expected: i64) {
    let row = row(&format!(r#"{{"startTime": {value}}}"#));
    assert_eq!(row.start_time, Some(expected));
}

#[test]
fn each_timestamp_key_lands_in_its_own_column() {
    let row = row(r#"{
            "startTime": 1700000000.0,
            "endTime": 1700000001.5,
            "completionStartTime": 1700000000.75
        }"#);
    assert_eq!(row.start_time, Some(1_700_000_000_000_000));
    assert_eq!(row.end_time, Some(1_700_000_001_500_000));
    assert_eq!(row.completion_start_time, Some(1_700_000_000_750_000));
}

#[rstest]
#[case::zero("0")]
#[case::negative("-1")]
#[case::negative_fraction("-0.5")]
#[case::null("null")]
fn a_nonpositive_or_null_timestamp_is_null(#[case] value: &str) {
    let row = row(&format!(r#"{{"startTime": {value}}}"#));
    assert_eq!(row.start_time, None);
}

#[test]
fn a_missing_timestamp_is_null() {
    let row = row("{}");
    assert_eq!(row.start_time, None);
    assert_eq!(row.end_time, None);
    assert_eq!(row.completion_start_time, None);
}

#[rstest]
#[case::text(r#""now""#)]
#[case::boolean("true")]
#[case::object(r#"{"seconds": 1}"#)]
fn a_wrongly_typed_timestamp_is_null(#[case] value: &str) {
    let row = row(&format!(r#"{{"startTime": {value}}}"#));
    assert_eq!(row.start_time, None);
}

#[test]
fn a_timestamp_past_the_microsecond_range_is_null() {
    let row = row(r#"{"startTime": 1e25}"#);
    assert_eq!(row.start_time, None);
}
