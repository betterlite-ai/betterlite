use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use crate::model_deprecation::dates::{
    decode_datetime_date, decode_deprecation_date, parse_json_date,
};

use super::support::{calendar_date, text};

#[rstest]
#[case::calendar_extended("2026-01-02", "2026-01-02")]
#[case::calendar_basic("20260102", "2026-01-02")]
#[case::week_extended_with_day("2026-W01-5", "2026-01-02")]
#[case::week_extended_monday("2026-W01-1", "2025-12-29")]
#[case::week_extended_tuesday("2026-W01-2", "2025-12-30")]
#[case::week_extended_wednesday("2026-W01-3", "2025-12-31")]
#[case::week_extended_thursday("2026-W01-4", "2026-01-01")]
#[case::week_extended_saturday("2026-W01-6", "2026-01-03")]
#[case::week_extended_sunday("2026-W01-7", "2026-01-04")]
#[case::week_basic_with_day("2026W015", "2026-01-02")]
#[case::week_extended_without_day("2026-W01", "2025-12-29")]
#[case::week_basic_without_day("2026W01", "2025-12-29")]
#[case::leap_day("2024-02-29", "2024-02-29")]
#[case::first_day("0001-01-01", "0001-01-01")]
#[case::last_day("9999-12-31", "9999-12-31")]
#[case::surrounding_spaces(" 2026-01-02\t", "2026-01-02")]
#[case::python_only_whitespace("\u{1c}2026-01-02\u{1f}", "2026-01-02")]
fn decodes_python_isoformats(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(
        decode_deprecation_date(source),
        Some(calendar_date(expected))
    );
}

#[rstest]
#[case::datetime_text("2026-01-02T00:00:00")]
#[case::datetime_with_offset("2026-01-02T00:00:00Z")]
#[case::single_digit_month("2026-1-2")]
#[case::single_digit_week("2026-W1-5")]
#[case::lowercase_week("2026-w01-5")]
#[case::year_zero("0000-01-01")]
#[case::year_zero_week("0000-W01-1")]
#[case::five_digit_year("10000-01-01")]
#[case::missing_leap_day("2026-02-29")]
#[case::missing_week_53("2021-W53-1")]
#[case::week_zero("2026-W00-1")]
#[case::weekday_zero("2026-W01-0")]
#[case::weekday_eight("2026-W01-8")]
#[case::month_zero("2026-00-10")]
#[case::month_thirteen("2026-13-01")]
#[case::slashes("2026/01/02")]
#[case::trailing_text("2026-01-02Z")]
#[case::leading_plus("+2026-01-02")]
#[case::month_only("2026-01")]
#[case::basic_month_only("202601")]
#[case::non_ascii_digits("٢٠٢٦-01-02")]
#[case::empty("")]
#[case::space_only("   ")]
#[case::truncated("2026-0")]
fn rejects_other_text(#[case] source: &str) {
    assert_eq!(decode_deprecation_date(source), None);
}

#[test]
fn datetime_dates_keep_the_local_calendar_date() {
    assert_eq!(
        decode_datetime_date("2026-05-01T23:30:00-05:00"),
        Some(calendar_date("2026-05-01"))
    );
    assert_eq!(
        decode_datetime_date("2026-05-02T01:30:00+09:00"),
        Some(calendar_date("2026-05-02"))
    );
    assert_eq!(
        decode_datetime_date("2026-05-03T10:00:00"),
        Some(calendar_date("2026-05-03"))
    );
    assert_eq!(
        decode_datetime_date("2026-05-04T00:00:00Z"),
        Some(calendar_date("2026-05-04"))
    );
    assert_eq!(decode_datetime_date("2026-05-03"), None);
}

#[test]
fn json_dates_only_read_text() {
    assert_eq!(
        parse_json_date(Some(&text("20260102"))),
        Some(calendar_date("2026-01-02"))
    );
    assert_eq!(parse_json_date(Some(&Json::Number(Number::from(5)))), None);
    assert_eq!(parse_json_date(Some(&Json::Null)), None);
    assert_eq!(parse_json_date(Some(&Json::List(Vec::new()))), None);
    assert_eq!(parse_json_date(None), None);
}
