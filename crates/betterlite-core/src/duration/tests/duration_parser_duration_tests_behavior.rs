#![cfg(test)]

use chrono::{DateTime, NaiveTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;

use crate::duration::error::{DurationError, DurationWarning};
use crate::duration::reset::{Reset, next_standardized_reset_time};
use crate::duration::seconds::{BudgetDuration, duration_in_seconds};
use crate::duration::spec::{ResetDurationSpec, parse_reset_duration};

const DATABASE: fn() -> TimeZoneDatabase = TimeZoneDatabase::from_env;

fn moment(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn reset_at(spec: &str, now: &str, zone: Option<&str>, time_of_day: &str) -> Reset {
    next_standardized_reset_time(
        spec,
        moment(now),
        zone,
        NaiveTime::parse_from_str(time_of_day, "%H:%M:%S%.f").unwrap(),
        &DATABASE(),
    )
    .unwrap()
}

fn budget(spec: &str, now: &str, zone: Option<&str>) -> BudgetDuration {
    duration_in_seconds(spec, moment(now), zone, &DATABASE()).unwrap()
}

fn budget_error(spec: &str) -> DurationError {
    duration_in_seconds(spec, moment("2024-06-15T12:00:00Z"), None, &DATABASE()).unwrap_err()
}

fn failure(error: &DurationError) -> &'static str {
    match error {
        DurationError::InvalidFormat { .. } => "invalid_format",
        DurationError::UnsupportedUnit { .. } => "unsupported_unit",
        DurationError::QuantityOverflow { .. } => "quantity_overflow",
        DurationError::SecondsOverflow { .. } => "seconds_overflow",
        DurationError::UnsupportedMonthInterval { .. } => "unsupported_month_interval",
        DurationError::CalendarOverflow => "calendar_overflow",
        DurationError::CalendarField { .. } => "calendar_field",
        DurationError::Calendar { .. } => "calendar",
        DurationError::Timestamp { .. } => "timestamp",
        DurationError::TimestampOutOfRange => "timestamp_out_of_range",
    }
}

#[rstest]
#[case::seconds("30s", 30)]
#[case::minutes("2m", 120)]
#[case::hours("3h", 10_800)]
#[case::days("4d", 345_600)]
#[case::weeks("2w", 1_209_600)]
fn duration_parser_duration_tests_behavior_fixed_units(#[case] spec: &str, #[case] expected: i64) {
    let result = budget(spec, "2024-06-15T12:00:00Z", None);
    assert_eq!(result.seconds, expected);
    assert!(result.warnings.is_empty());
}

#[rstest]
#[case::hourly("hourly", 3_600)]
#[case::daily("daily", 86_400)]
#[case::weekly("weekly", 604_800)]
#[case::monthly_is_thirty_days("monthly", 2_592_000)]
#[case::trimmed(" Daily ", 86_400)]
#[case::uppercased("MONTHLY", 2_592_000)]
fn duration_parser_duration_tests_behavior_word_aliases(#[case] spec: &str, #[case] expected: i64) {
    assert_eq!(budget(spec, "2024-06-15T12:00:00Z", None).seconds, expected);
}

#[rstest]
#[case::month_end_clamps_the_day("1mo", "2024-01-31T12:30:45.123456Z", "UTC", 2_505_600)]
#[case::local_timezone_picks_the_day("1mo", "2024-02-01T03:00:00Z", "America/New_York", 2_505_600)]
#[case::dst_shift_is_not_counted("1mo", "2024-03-09T12:00:00Z", "America/New_York", 2_678_400)]
#[case::leap_year_clamps_to_the_28th("12mo", "2024-02-29T00:00:00Z", "UTC", 31_536_000)]
#[case::four_leap_years_keep_the_day("48mo", "2020-02-29T00:00:00Z", "UTC", 126_230_400)]
#[case::december_rolls_the_year("1mo", "2024-12-31T00:00:00Z", "UTC", 2_678_400)]
#[case::zero_months_is_now("0mo", "2024-06-15T12:00:00Z", "UTC", 0)]
fn duration_parser_duration_tests_behavior_calendar_months(
    #[case] spec: &str,
    #[case] now: &str,
    #[case] zone: &str,
    #[case] expected: i64,
) {
    assert_eq!(budget(spec, now, Some(zone)).seconds, expected);
}

#[test]
fn duration_parser_duration_tests_behavior_monthly_alias_differs_from_calendar_month() {
    assert_eq!(
        budget("monthly", "2024-01-31T00:00:00Z", None).seconds,
        2_592_000
    );
    assert_eq!(
        budget("1mo", "2024-01-31T00:00:00Z", None).seconds,
        2_505_600
    );
}

#[rstest]
#[case::empty("", "invalid_format")]
#[case::letters("abc", "invalid_format")]
#[case::leading_space(" 5m", "invalid_format")]
#[case::negative("-5m", "invalid_format")]
#[case::unit_only("mo", "invalid_format")]
#[case::bare_number("5", "unsupported_unit")]
#[case::unknown_letter("5x", "unsupported_unit")]
#[case::uppercase_unit("5M", "unsupported_unit")]
fn duration_parser_duration_tests_behavior_invalid_forms(
    #[case] spec: &str,
    #[case] expected: &str,
) {
    let error = budget_error(spec);
    assert_eq!(failure(&error), expected);
}

#[rstest]
#[case::huge_quantity("999999999999999999999999s", "quantity_overflow")]
#[case::past_u64("18446744073709551616s", "quantity_overflow")]
#[case::past_i64_seconds("9223372036854775808s", "seconds_overflow")]
#[case::day_product_past_i64("106751991167301d", "seconds_overflow")]
#[case::month_past_the_calendar("100000mo", "calendar")]
#[case::month_past_i16("999999999999mo", "calendar_field")]
fn duration_parser_duration_tests_behavior_overflow_is_typed(
    #[case] spec: &str,
    #[case] expected: &str,
) {
    let error = budget_error(spec);
    assert_eq!(failure(&error), expected);
}

#[rstest]
#[case::trailing_words("5m trailing", 300, " trailing")]
#[case::trailing_letters("10dabc", 864_000, "abc")]
#[case::prefix_unit("2hours", 7_200, "ours")]
#[case::trailing_after_month("3mo extra", 7_776_000, " extra")]
fn duration_parser_duration_tests_behavior_trailing_input_is_reported(
    #[case] spec: &str,
    #[case] expected: i64,
    #[case] rest: &str,
) {
    let result = budget(spec, "2024-01-31T00:00:00Z", None);
    assert_eq!(result.seconds, expected);
    assert_eq!(
        result.warnings,
        vec![DurationWarning::TrailingInput {
            rest: rest.to_owned()
        }]
    );
}

#[test]
fn duration_parser_duration_tests_behavior_invalid_timezone_falls_back_to_utc() {
    let result = budget("1mo", "2024-01-31T00:00:00Z", Some("Not/AZone"));
    assert_eq!(result.seconds, 2_505_600);
    assert_eq!(
        result.warnings,
        vec![DurationWarning::InvalidTimezone {
            name: "Not/AZone".to_owned()
        }]
    );
    assert!(
        budget("1mo", "2024-01-31T00:00:00Z", None)
            .warnings
            .is_empty()
    );
}

#[rstest]
#[case::second(
    "30s",
    "2024-01-01T00:00:10Z",
    "2024-01-01T00:00:30Z",
    "2024-01-01T00:00:30"
)]
#[case::minute(
    "15m",
    "2024-01-01T10:07:45Z",
    "2024-01-01T10:15:00Z",
    "2024-01-01T10:15:00"
)]
#[case::hour(
    "1h",
    "2024-01-01T10:30:00Z",
    "2024-01-01T11:00:00Z",
    "2024-01-01T11:00:00"
)]
#[case::overnight_hour(
    "25h",
    "2024-01-01T00:00:00Z",
    "2024-01-02T01:00:00Z",
    "2024-01-02T01:00:00"
)]
#[case::day(
    "1d",
    "2024-01-01T10:00:00Z",
    "2024-01-02T00:00:00Z",
    "2024-01-02T00:00:00"
)]
#[case::custom_days(
    "2d",
    "2024-01-01T10:00:00Z",
    "2024-01-03T00:00:00Z",
    "2024-01-03T00:00:00"
)]
#[case::weekly(
    "7d",
    "2024-01-03T10:00:00Z",
    "2024-01-08T00:00:00Z",
    "2024-01-08T00:00:00"
)]
#[case::weekly_from_monday(
    "7d",
    "2024-01-01T00:00:00Z",
    "2024-01-08T00:00:00Z",
    "2024-01-08T00:00:00"
)]
#[case::thirty_days_is_monthly(
    "30d",
    "2024-01-15T10:00:00Z",
    "2024-02-01T00:00:00Z",
    "2024-02-01T00:00:00"
)]
#[case::month(
    "1mo",
    "2024-01-15T10:00:00Z",
    "2024-02-01T00:00:00Z",
    "2024-02-01T00:00:00"
)]
#[case::month_from_the_first(
    "1mo",
    "2024-01-01T00:00:00Z",
    "2024-02-01T00:00:00Z",
    "2024-02-01T00:00:00"
)]
fn duration_parser_duration_tests_behavior_reset_units(
    #[case] spec: &str,
    #[case] now: &str,
    #[case] expected_instant: &str,
    #[case] expected_civil: &str,
) {
    let result = reset_at(spec, now, None, "00:00:00");
    assert_eq!(result.instant, moment(expected_instant));
    assert_eq!(result.civil.to_string(), expected_civil);
    assert!(result.warnings.is_empty());
}

#[rstest]
#[case::days("0d")]
#[case::weeks("0w")]
#[case::hours("0h")]
#[case::minutes("0m")]
#[case::seconds("0s")]
fn duration_parser_duration_tests_behavior_zero_intervals_return_current(#[case] spec: &str) {
    let now = moment("2024-01-01T03:00:00.500000Z");
    let result = next_standardized_reset_time(
        spec,
        now,
        None,
        NaiveTime::parse_from_str("06:00:00", "%H:%M:%S").unwrap(),
        &DATABASE(),
    )
    .unwrap();
    assert_eq!(result.instant, now);
}

#[rstest]
#[case::two("2mo", 2)]
#[case::zero("0mo", 0)]
#[case::twelve("12mo", 12)]
fn duration_parser_duration_tests_behavior_rejects_month_intervals_but_one(
    #[case] spec: &str,
    #[case] quantity: u64,
) {
    let error = next_standardized_reset_time(
        spec,
        moment("2024-01-15T00:00:00Z"),
        None,
        NaiveTime::MIN,
        &DATABASE(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        DurationError::UnsupportedMonthInterval { quantity: actual } if actual == quantity
    ));
}

#[rstest]
#[case::same_day("1d", "2024-01-01T03:00:00Z", None, "06:00:00", "2024-01-01T06:00:00Z")]
#[case::next_day("1d", "2024-01-01T10:00:00Z", None, "06:00:00", "2024-01-02T06:00:00Z")]
#[case::custom_days_ignore_the_time(
    "2d",
    "2024-01-01T10:00:00Z",
    None,
    "06:00:00",
    "2024-01-03T06:00:00Z"
)]
#[case::weekly_same_monday("7d", "2024-01-01T03:00:00Z", None, "06:00:00", "2024-01-01T06:00:00Z")]
#[case::monthly(
    "1mo",
    "2024-01-15T10:00:00Z",
    None,
    "06:00:00",
    "2024-02-01T06:00:00Z"
)]
#[case::fractional(
    "1d",
    "2024-01-01T03:00:00Z",
    None,
    "00:00:00.500000",
    "2024-01-02T00:00:00.500000Z"
)]
#[case::gap_day_handler(
    "1d",
    "2024-03-09T12:00:00Z",
    Some("America/New_York"),
    "02:30:00",
    "2024-03-10T07:30:00Z"
)]
#[case::gap_day_handler_on_the_gap_day(
    "1d",
    "2024-03-10T06:30:00Z",
    Some("America/New_York"),
    "02:30:00",
    "2024-03-10T07:30:00Z"
)]
#[case::gap_hour_handler(
    "1h",
    "2024-03-10T06:30:00Z",
    Some("America/New_York"),
    "00:00:00",
    "2024-03-10T07:00:00Z"
)]
#[case::gap_minute_handler(
    "30m",
    "2024-03-10T06:45:00Z",
    Some("America/New_York"),
    "00:00:00",
    "2024-03-10T07:00:00Z"
)]
#[case::first_fold_occurrence(
    "1d",
    "2024-11-03T04:30:00Z",
    Some("America/New_York"),
    "01:30:00",
    "2024-11-03T05:30:00Z"
)]
#[case::second_fold_occurrence(
    "1d",
    "2024-11-03T06:00:00Z",
    Some("America/New_York"),
    "01:30:00",
    "2024-11-03T06:30:00Z"
)]
#[case::just_after_the_fold(
    "1d",
    "2024-11-03T05:30:00Z",
    Some("America/New_York"),
    "01:30:00",
    "2024-11-04T06:30:00Z"
)]
fn duration_parser_duration_tests_behavior_reset_instants(
    #[case] spec: &str,
    #[case] now: &str,
    #[case] zone: Option<&str>,
    #[case] time_of_day: &str,
    #[case] expected: &str,
) {
    let result = reset_at(spec, now, zone, time_of_day);
    assert_eq!(result.instant, moment(expected));
}

#[rstest]
#[case::unparsable_letters("abc", DurationWarning::UnrecognizedResetDuration { spec: "abc".to_owned() })]
#[case::unparsable_digits("5", DurationWarning::UnrecognizedResetDuration { spec: "5".to_owned() })]
#[case::unparsable_uppercase("5M", DurationWarning::UnrecognizedResetDuration { spec: "5M".to_owned() })]
#[case::unparsable_leading_space(" 5m", DurationWarning::UnrecognizedResetDuration { spec: " 5m".to_owned() })]
#[case::unknown_letter("5x", DurationWarning::UnknownResetUnit { unit: "x".to_owned() })]
#[case::unknown_word("5mox", DurationWarning::UnknownResetUnit { unit: "mox".to_owned() })]
#[case::unknown_plural("2hours", DurationWarning::UnknownResetUnit { unit: "hours".to_owned() })]
#[case::unknown_huge("999999999999999999999999x", DurationWarning::UnknownResetUnit { unit: "x".to_owned() })]
fn duration_parser_duration_tests_behavior_reset_fallbacks(
    #[case] spec: &str,
    #[case] warning: DurationWarning,
) {
    let result = reset_at(spec, "2024-01-01T10:00:00Z", None, "06:00:00");
    assert_eq!(result.instant, moment("2024-01-02T00:00:00Z"));
    assert_eq!(result.warnings, vec![warning]);
}

#[test]
fn duration_parser_duration_tests_behavior_invalid_reset_timezone_falls_back_to_utc() {
    let result = reset_at("1d", "2024-01-01T10:00:00Z", Some("Not/AZone"), "00:00:00");
    assert_eq!(result.instant, moment("2024-01-02T00:00:00Z"));
    assert_eq!(
        result.warnings,
        vec![DurationWarning::InvalidTimezone {
            name: "Not/AZone".to_owned()
        }]
    );
    assert!(
        reset_at("1d", "2024-01-01T10:00:00Z", None, "00:00:00")
            .warnings
            .is_empty()
    );
}

#[test]
fn duration_parser_duration_tests_behavior_reset_trailing_input_is_reported() {
    let result = reset_at("5m!", "2024-01-01T10:00:00Z", None, "00:00:00");
    assert_eq!(result.instant, moment("2024-01-01T10:05:00Z"));
    assert_eq!(
        result.warnings,
        vec![DurationWarning::TrailingInput {
            rest: "!".to_owned()
        }]
    );
}

#[test]
fn duration_parser_duration_tests_behavior_reset_parser_keeps_the_prefix() {
    let parsed = parse_reset_duration("5m!").unwrap();
    assert!(matches!(parsed, ResetDurationSpec::Known { .. }));
}

#[test]
fn duration_parser_duration_tests_behavior_injected_time_decides_the_reset() {
    let first = reset_at("1d", "2024-01-01T10:00:00Z", None, "00:00:00");
    let second = reset_at("1d", "2024-06-01T10:00:00Z", None, "00:00:00");
    assert_eq!(first.instant, moment("2024-01-02T00:00:00Z"));
    assert_eq!(second.instant, moment("2024-06-02T00:00:00Z"));
}
