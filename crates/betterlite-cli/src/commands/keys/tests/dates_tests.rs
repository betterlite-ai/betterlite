use chrono::{NaiveDate, NaiveTime};
use rstest::rstest;

use super::super::dates;

#[rstest]
#[case::date_only("2024-05-01", 0, 0)]
#[case::date_and_minute("2024-05-01_23:30", 23, 30)]
fn dates_parses_the_two_created_since_formats(
    #[case] text: &str,
    #[case] hour: u32,
    #[case] minute: u32,
) {
    let expected = NaiveDate::from_ymd_opt(2024, 5, 1)
        .unwrap()
        .and_hms_opt(hour, minute, 0)
        .unwrap();
    assert_eq!(dates::parse_created_since(text).unwrap(), expected);
}

#[rstest]
#[case::empty("")]
#[case::trailing_text("2024-05-01_10:30_extra")]
#[case::text("yesterday")]
fn dates_rejects_other_created_since_text(#[case] text: &str) {
    assert!(dates::parse_created_since(text).is_err(), "{text}");
}

#[rstest]
#[case::plain("2024-05-01T23:30:00", 0)]
#[case::fraction("2024-05-01T23:30:00.123456", 123_456)]
#[case::utc("2024-05-01T23:30:00Z", 0)]
#[case::offset("2024-05-01T23:30:00+02:00", 0)]
#[case::negative_offset("2024-05-01T23:30:00-02:00", 0)]
fn dates_reads_a_created_at_wall_time(#[case] text: &str, #[case] micros: u32) {
    let expected = NaiveDate::from_ymd_opt(2024, 5, 1)
        .unwrap()
        .and_hms_micro_opt(23, 30, 0, micros)
        .unwrap();
    assert_eq!(dates::parse_created_at(text).unwrap(), expected);
}

#[test]
fn dates_drops_the_offset_without_converting() {
    let parsed = dates::parse_created_at("2024-05-01T23:30:00-02:00").unwrap();
    let may_first = NaiveDate::from_ymd_opt(2024, 5, 1).unwrap();
    assert_eq!(parsed.date(), may_first);
    assert_eq!(parsed.time(), NaiveTime::from_hms_opt(23, 30, 0).unwrap());
}

#[rstest]
#[case::text("yesterday")]
#[case::date_only("2024-05-01")]
#[case::empty("")]
fn dates_rejects_another_created_at_string(#[case] text: &str) {
    assert!(dates::parse_created_at(text).is_err(), "{text}");
}
