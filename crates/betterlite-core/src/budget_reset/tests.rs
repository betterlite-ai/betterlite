#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str;

use betterlite_config::sections::budget_reset::clock::decode_reset_clock;
use betterlite_config::sections::budget_reset::{BudgetResetSettings, get_budget_reset_settings};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;
use serde_json::json;

use super::{
    ResetClocks, budget_duration_error, compute_budget_reset_at, is_persistable_budget_duration,
};
use crate::duration::error::{DurationError, DurationWarning};
use crate::duration::seconds::duration_in_seconds;

const DATABASE: fn() -> TimeZoneDatabase = TimeZoneDatabase::from_env;
const MONTHLY_ALIAS_SECONDS: i64 = 2_592_000;

fn moment(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn settings(timezone: &str, reset_time: &str) -> BudgetResetSettings {
    get_budget_reset_settings(&json!(timezone), &json!(reset_time)).unwrap()
}

fn clocks<'zone>(reset: &str, calendar: &str, zone: Option<&'zone str>) -> ResetClocks<'zone> {
    ResetClocks {
        reset_now: moment(reset),
        calendar_now: moment(calendar),
        calendar_timezone: zone,
    }
}

#[test]
fn current_settings_snapshot_used() {
    let now = moment("2024-01-01T10:00:00Z");
    let morning = compute_budget_reset_at("1d", &settings("UTC", "06:00"), now, &DATABASE());
    let evening = compute_budget_reset_at("1d", &settings("UTC", "18:00"), now, &DATABASE());
    assert_eq!(morning.unwrap().instant, moment("2024-01-02T06:00:00Z"));
    assert_eq!(evening.unwrap().instant, moment("2024-01-01T18:00:00Z"));
    let early = compute_budget_reset_at("1d", &settings("UTC", "06:00"), now, &DATABASE());
    let late = compute_budget_reset_at("1d", &settings("UTC", "18:00"), now, &DATABASE());
    assert_ne!(early.unwrap().instant, late.unwrap().instant);
}

#[test]
fn configured_zone_and_clock_forwarded() {
    let reset = compute_budget_reset_at(
        "1d",
        &settings("Asia/Kolkata", "06:00"),
        moment("2024-01-01T10:00:00Z"),
        &DATABASE(),
    )
    .unwrap();
    assert_eq!(reset.instant, moment("2024-01-02T00:30:00Z"));
    assert_eq!(reset.civil.to_string(), "2024-01-02T06:00:00");
}

#[test]
fn invalid_zone_silent_utc() {
    let configured = settings("Not/AZone", "00:00");
    assert_eq!(configured.timezone, "Not/AZone");
    let reset = compute_budget_reset_at(
        "1d",
        &configured,
        moment("2024-01-01T10:00:00Z"),
        &DATABASE(),
    )
    .unwrap();
    assert_eq!(reset.instant, moment("2024-01-02T00:00:00Z"));
    assert_eq!(reset.civil.to_string(), "2024-01-02T00:00:00");
}

#[test]
fn malformed_duration_warning_midnight() {
    let reset = compute_budget_reset_at(
        "abc",
        &settings("UTC", "06:00"),
        moment("2024-01-01T10:00:00Z"),
        &DATABASE(),
    )
    .unwrap();
    assert_eq!(reset.instant, moment("2024-01-02T00:00:00Z"));
    assert!(
        reset
            .warnings
            .iter()
            .any(|warning| matches!(warning, DurationWarning::UnrecognizedResetDuration { .. }))
    );
}

#[test]
fn unknown_reset_unit_silent_midnight() {
    let reset = compute_budget_reset_at(
        "1hour",
        &settings("UTC", "06:00"),
        moment("2024-01-01T10:00:00Z"),
        &DATABASE(),
    )
    .unwrap();
    assert_eq!(reset.instant, moment("2024-01-02T00:00:00Z"));
    assert!(
        reset
            .warnings
            .iter()
            .any(|warning| matches!(warning, DurationWarning::UnknownResetUnit { .. }))
    );
    assert!(
        !reset
            .warnings
            .iter()
            .any(|warning| matches!(warning, DurationWarning::UnrecognizedResetDuration { .. }))
    );
}

#[rstest]
#[case::seconds("0s")]
#[case::minutes("0m")]
#[case::hours("0h")]
#[case::days("0d")]
#[case::weeks("0w")]
fn direct_zero_interval_returns_now(#[case] duration: &str) {
    let now = moment("2024-01-01T03:00:00.500000Z");
    let computed = compute_budget_reset_at(duration, &settings("UTC", "06:00"), now, &DATABASE());
    let reset = computed.unwrap();
    assert_eq!(reset.instant, now);
    assert!(reset.warnings.is_empty());
}

#[rstest]
#[case::two("2mo")]
#[case::zero("0mo")]
#[case::twelve("12mo")]
fn monthly_interval_other_than_one_error(#[case] duration: &str) {
    let error = compute_budget_reset_at(
        duration,
        &settings("UTC", "00:00"),
        moment("2024-01-15T00:00:00Z"),
        &DATABASE(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        DurationError::UnsupportedMonthInterval { .. }
    ));
}

#[rstest]
#[case::day("1d", true)]
#[case::week("7d", true)]
#[case::days("30d", true)]
#[case::month("1mo", true)]
#[case::hour("1h", false)]
#[case::minutes("30m", false)]
#[case::seconds("30s", false)]
fn time_of_day_calendar_units_only(#[case] duration: &str, #[case] changes: bool) {
    let now = moment("2024-01-15T10:00:00Z");
    let early =
        compute_budget_reset_at(duration, &settings("UTC", "06:00"), now, &DATABASE()).unwrap();
    let late =
        compute_budget_reset_at(duration, &settings("UTC", "18:00"), now, &DATABASE()).unwrap();
    assert_eq!(early.instant != late.instant, changes, "{duration}");
}

#[test]
fn offset_preserved() {
    let reset = compute_budget_reset_at(
        "1d",
        &settings("Asia/Kolkata", "06:00"),
        moment("2024-01-01T10:00:00Z"),
        &DATABASE(),
    )
    .unwrap();
    assert_eq!(reset.instant, moment("2024-01-02T00:30:00Z"));
    assert_eq!(reset.civil.to_string(), "2024-01-02T06:00:00");
    assert_ne!(reset.instant, moment("2024-01-02T06:00:00Z"));
}

#[test]
fn separate_local_and_utc_clock_samples() {
    let configured = settings("UTC", "00:00");
    let calendar_overflow = clocks("2024-06-15T12:00:00Z", "9999-12-15T00:00:00Z", None);
    assert!(!is_persistable_budget_duration(
        "1mo",
        &configured,
        calendar_overflow,
        &DATABASE()
    ));
    let reset_overflow = clocks("9999-12-15T00:00:00Z", "2024-06-15T12:00:00Z", None);
    assert!(!is_persistable_budget_duration(
        "1mo",
        &configured,
        reset_overflow,
        &DATABASE()
    ));
    let both_valid = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    assert!(is_persistable_budget_duration(
        "1mo",
        &configured,
        both_valid,
        &DATABASE()
    ));
    let month = duration_in_seconds("1mo", both_valid.calendar_now, None, &DATABASE()).unwrap();
    assert!(month.seconds > 0);
}

#[test]
fn absent_duration_skips_everything() {
    let result = budget_duration_error(
        None,
        &settings("UTC", "06:00"),
        clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None),
        &DATABASE(),
    );
    assert_eq!(result, None);
}

#[rstest]
#[case::hour("1h")]
#[case::day("24h")]
#[case::week("7d")]
#[case::days("30d")]
#[case::monthly_alias("monthly")]
#[case::calendar_month("1mo")]
fn positive_duration_accepted(#[case] duration: &str) {
    let configured = settings("UTC", "00:00");
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    assert!(is_persistable_budget_duration(
        duration,
        &configured,
        clock,
        &DATABASE()
    ));
    assert_eq!(
        budget_duration_error(Some(duration), &configured, clock, &DATABASE()),
        None
    );
}

#[rstest]
#[case::seconds("0s")]
#[case::minutes("0m")]
#[case::hours("0h")]
#[case::days("0d")]
#[case::weeks("0w")]
#[case::months("0mo")]
fn nonpositive_short_circuits_settings(#[case] duration: &str) {
    let configured = settings("UTC", "06:00");
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    let persistable = is_persistable_budget_duration(duration, &configured, clock, &DATABASE());
    assert!(!persistable);
}

#[test]
fn two_month_duration_rejected() {
    let configured = settings("UTC", "00:00");
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    assert!(!is_persistable_budget_duration(
        "2mo",
        &configured,
        clock,
        &DATABASE()
    ));
    assert!(budget_duration_error(Some("2mo"), &configured, clock, &DATABASE()).is_some());
    let error =
        compute_budget_reset_at("2mo", &configured, clock.reset_now, &DATABASE()).unwrap_err();
    assert!(matches!(
        error,
        DurationError::UnsupportedMonthInterval { .. }
    ));
}

#[rstest]
#[case::unsupported_unit("5x", true)]
#[case::unparsable("abc", true)]
#[case::two_month("2mo", false)]
#[case::twelve_month("12mo", false)]
fn seconds_or_reset_value_error_rejected(#[case] duration: &str, #[case] seconds_step: bool) {
    let configured = settings("UTC", "00:00");
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    let seconds = duration_in_seconds(
        duration,
        clock.calendar_now,
        clock.calendar_timezone,
        &DATABASE(),
    );
    assert_eq!(seconds.is_err(), seconds_step, "{duration}");
    assert!(!is_persistable_budget_duration(
        duration,
        &configured,
        clock,
        &DATABASE()
    ));
}

#[rstest]
#[case::quantity("99999999999999999999d")]
#[case::reset_range("100000000d")]
fn overflow_rejected(#[case] duration: &str) {
    let configured = settings("UTC", "00:00");
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    let failure =
        compute_budget_reset_at(duration, &configured, clock.reset_now, &DATABASE()).unwrap_err();
    assert!(matches!(
        failure,
        DurationError::QuantityOverflow { .. } | DurationError::Calendar { .. }
    ));
    assert!(!is_persistable_budget_duration(
        duration,
        &configured,
        clock,
        &DATABASE()
    ));
}

#[rstest]
#[case::zero("0h")]
#[case::unparsable("abc")]
#[case::unsupported_unit("5x")]
#[case::two_month("2mo")]
#[case::twelve_month("12mo")]
#[case::overflow("99999999999999999999d")]
#[case::negative("-5m")]
fn rejected_durations_report_the_diagnostic(#[case] duration: &str) {
    let configured = settings("UTC", "06:00");
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    assert!(!is_persistable_budget_duration(
        duration,
        &configured,
        clock,
        &DATABASE()
    ));
    assert!(budget_duration_error(Some(duration), &configured, clock, &DATABASE()).is_some());
}

#[test]
fn positive_prefix_unknown_reset_unit_accepted() {
    let configured = settings("UTC", "00:00");
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    assert!(is_persistable_budget_duration(
        "1hour",
        &configured,
        clock,
        &DATABASE()
    ));
    assert_eq!(
        budget_duration_error(Some("1hour"), &configured, clock, &DATABASE()),
        None
    );
    let reset =
        compute_budget_reset_at("1hour", &configured, clock.reset_now, &DATABASE()).unwrap();
    assert_eq!(reset.instant, moment("2024-06-16T00:00:00Z"));
}

#[test]
fn monthly_alias_differs_from_calendar_month() {
    let database = DATABASE();
    let now = moment("2024-01-31T00:00:00Z");
    let alias = duration_in_seconds("monthly", now, None, &database).unwrap();
    assert_eq!(alias.seconds, MONTHLY_ALIAS_SECONDS);
    let calendar = duration_in_seconds("1mo", now, None, &database).unwrap();
    assert_eq!(calendar.seconds, 2_505_600);
    assert_ne!(alias.seconds, calendar.seconds);
}

#[rstest]
#[case::hour("1h")]
#[case::day("24h")]
#[case::week("7d")]
#[case::days("30d")]
fn invalid_duration_names_field_and_examples(#[case] example: &str) {
    let message = budget_duration_error(
        Some("abc"),
        &settings("UTC", "00:00"),
        clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None),
        &DATABASE(),
    )
    .unwrap();
    assert!(message.contains("budget_duration"));
    assert!(message.contains(example));
}

#[rstest]
fn replays_the_reset_clock_corpus(#[files("fuzz/corpus/budget_reset_clock/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    drop(decode_reset_clock(text));
}
