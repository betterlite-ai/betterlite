#![cfg(test)]

use super::parse_date;
use rstest::rstest;

#[rstest]
#[case::rfc2822_gmt("Thu, 12 Mar 2026 10:00:00 GMT", "2026-03-12")]
#[case::mismatched_weekday_is_ignored("Fri, 12 Mar 2026 10:00:00 GMT", "2026-03-12")]
#[case::missing_weekday_and_timezone("12 Mar 2026 10:00:00", "2026-03-12")]
#[case::numeric_timezone_offset("Thu, 12 Mar 2026 10:00:00 +0530", "2026-03-12")]
#[case::two_digit_year("12 Mar 26 10:00:00 GMT", "2026-03-12")]
fn formats_valid_dates(#[case] raw: &str, #[case] expected: &str) {
    assert_eq!(parse_date(raw), expected);
}

#[rstest]
#[case::unparsable_text("not-a-date")]
#[case::invalid_day_of_month("Thu, 32 Mar 2026 10:00:00 GMT")]
#[case::invalid_hour("Thu, 12 Mar 2026 25:00:00 GMT")]
#[case::invalid_timezone("Thu, 12 Mar 2026 10:00:00 +2500")]
#[case::year_outside_python_range("Thu, 12 Mar 12026 10:00:00 GMT")]
#[case::empty_string("")]
fn keeps_raw_text_on_failure(#[case] raw: &str) {
    assert_eq!(parse_date(raw), raw);
}
