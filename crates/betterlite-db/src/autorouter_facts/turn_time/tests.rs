#![cfg(test)]

use std::fs::read;
use std::path::PathBuf;

use chrono::{NaiveDate, NaiveDateTime};
use rstest::rstest;

use super::{decode_turn_time, turn_time_utc};

fn expected(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").unwrap()
}

#[test]
fn a_utc_timestamp_is_already_naive_utc() {
    assert_eq!(
        turn_time_utc("2024-01-02T03:04:05Z"),
        Some(expected("2024-01-02T03:04:05"))
    );
    assert_eq!(
        turn_time_utc("2024-01-02T03:04:05+00:00"),
        Some(expected("2024-01-02T03:04:05"))
    );
}

#[test]
fn a_fractional_timestamp_keeps_its_fraction() {
    assert_eq!(
        turn_time_utc("2024-01-02T03:04:05.123456Z"),
        Some(expected("2024-01-02T03:04:05.123456"))
    );
    assert_eq!(
        turn_time_utc("2024-01-02T03:04:05.5Z"),
        Some(expected("2024-01-02T03:04:05.5"))
    );
}

#[test]
fn an_aware_timestamp_converts_to_utc() {
    assert_eq!(
        turn_time_utc("2024-01-02T03:04:05+02:00"),
        Some(expected("2024-01-02T01:04:05"))
    );
    assert_eq!(
        turn_time_utc("2024-01-02T03:04:05.250-05:30"),
        Some(expected("2024-01-02T08:34:05.250"))
    );
    assert_eq!(
        turn_time_utc("2024-01-02T03:04:05+0200"),
        Some(expected("2024-01-02T01:04:05"))
    );
}

#[test]
fn a_naive_timestamp_stays_as_sent() {
    assert_eq!(
        turn_time_utc("2024-01-02T03:04:05"),
        Some(expected("2024-01-02T03:04:05"))
    );
    assert_eq!(
        turn_time_utc("2024-01-02 03:04:05"),
        Some(expected("2024-01-02T03:04:05"))
    );
}

#[test]
fn a_timestamp_without_seconds_or_a_date_reads_the_minute_or_midnight() {
    assert_eq!(
        turn_time_utc("2024-01-02T03:04"),
        Some(expected("2024-01-02T03:04:00"))
    );
    assert_eq!(
        turn_time_utc("2024-01-02 03:04"),
        Some(expected("2024-01-02T03:04:00"))
    );
    assert_eq!(
        turn_time_utc("2024-01-02"),
        Some(
            NaiveDate::from_ymd_opt(2024, 1, 2)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
        )
    );
}

#[rstest]
#[case::empty("")]
#[case::text("not a time")]
#[case::month_thirteen("2024-13-02T03:04:05")]
#[case::hour_twenty_four("2024-01-02T24:00:00")]
#[case::offset_past_a_day("2024-01-02T03:04:05+25:00")]
#[case::slashes("2024/01/02 03:04:05")]
fn an_unparsable_timestamp_reads_none(#[case] text: &str) {
    assert_eq!(turn_time_utc(text), None);
}

#[test]
fn bytes_that_are_not_utf8_read_none() {
    assert_eq!(decode_turn_time(&[0xff, 0xfe]), None);
    assert_eq!(
        decode_turn_time(b"2024-01-02T03:04:05Z"),
        Some(expected("2024-01-02T03:04:05"))
    );
}

#[rstest]
fn replays_the_turn_time_corpus(#[files("fuzz/corpus/turn_time/*")] path: PathBuf) {
    drop(decode_turn_time(&read(path).unwrap()));
}
