#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str;

use betterlite_config::sections::budget_reset::clock::decode_reset_clock;
use betterlite_config::sections::budget_reset::{
    BudgetResetSettings, BudgetResetSettingsError, get_budget_reset_settings,
};
use betterlite_config::sections::settings::JsonValue;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;
use serde_json::json;

use super::{
    BudgetResetError, RawBudgetResetSettings, ResetClocks, budget_duration_error,
    compute_budget_reset_at, get_budget_reset_time, is_persistable_budget_duration,
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

fn raw<'values>(
    timezone: &'values JsonValue,
    reset_time: &'values JsonValue,
) -> RawBudgetResetSettings<'values> {
    RawBudgetResetSettings {
        timezone,
        reset_time_of_day: reset_time,
    }
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
    let timezone = json!("UTC");
    let early_time = json!("06:00");
    let late_time = json!("18:00");
    let early = get_budget_reset_time("1d", raw(&timezone, &early_time), now, &DATABASE());
    let late = get_budget_reset_time("1d", raw(&timezone, &late_time), now, &DATABASE());
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
    let timezone = json!("Not/AZone");
    let via_settings = get_budget_reset_time(
        "1d",
        raw(&timezone, &json!(null)),
        moment("2024-01-01T10:00:00Z"),
        &DATABASE(),
    )
    .unwrap();
    assert_eq!(via_settings.instant, reset.instant);
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
    let timezone = json!(null);
    let reset_time = json!(null);
    let database = DATABASE();
    let calendar_overflow = clocks("2024-06-15T12:00:00Z", "9999-12-15T00:00:00Z", None);
    assert!(
        !is_persistable_budget_duration(
            "1mo",
            raw(&timezone, &reset_time),
            calendar_overflow,
            &database
        )
        .unwrap()
    );
    let reset_overflow = clocks("9999-12-15T00:00:00Z", "2024-06-15T12:00:00Z", None);
    assert!(
        !is_persistable_budget_duration(
            "1mo",
            raw(&timezone, &reset_time),
            reset_overflow,
            &database
        )
        .unwrap()
    );
    let both_valid = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    assert!(
        is_persistable_budget_duration("1mo", raw(&timezone, &reset_time), both_valid, &database)
            .unwrap()
    );
    let month = duration_in_seconds("1mo", both_valid.calendar_now, None, &database).unwrap();
    assert!(month.seconds > 0);
}

#[test]
fn absent_duration_skips_everything() {
    let timezone = json!(1_i32);
    let reset_time = json!(0_i32);
    let result = budget_duration_error(
        None,
        raw(&timezone, &reset_time),
        clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None),
        &DATABASE(),
    );
    assert_eq!(result.unwrap(), None);
}

#[rstest]
#[case::hour("1h")]
#[case::day("24h")]
#[case::week("7d")]
#[case::days("30d")]
#[case::monthly_alias("monthly")]
#[case::calendar_month("1mo")]
fn positive_duration_accepted(#[case] duration: &str) {
    let timezone = json!(null);
    let reset_time = json!(null);
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    assert!(
        is_persistable_budget_duration(duration, raw(&timezone, &reset_time), clock, &DATABASE())
            .unwrap()
    );
    assert_eq!(
        budget_duration_error(
            Some(duration),
            raw(&timezone, &reset_time),
            clock,
            &DATABASE()
        )
        .unwrap(),
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
    let timezone = json!(1_i32);
    let reset_time = json!(false);
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    let persistable =
        is_persistable_budget_duration(duration, raw(&timezone, &reset_time), clock, &DATABASE());
    assert!(!persistable.unwrap());
}

#[test]
fn two_month_duration_rejected() {
    let timezone = json!(null);
    let reset_time = json!(null);
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    assert!(
        !is_persistable_budget_duration("2mo", raw(&timezone, &reset_time), clock, &DATABASE())
            .unwrap()
    );
    assert!(
        budget_duration_error(Some("2mo"), raw(&timezone, &reset_time), clock, &DATABASE())
            .unwrap()
            .is_some()
    );
    let error = get_budget_reset_time(
        "2mo",
        raw(&timezone, &reset_time),
        clock.reset_now,
        &DATABASE(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        BudgetResetError::Duration(DurationError::UnsupportedMonthInterval { .. })
    ));
}

#[rstest]
#[case::unsupported_unit("5x", true)]
#[case::unparsable("abc", true)]
#[case::two_month("2mo", false)]
#[case::twelve_month("12mo", false)]
fn seconds_or_reset_value_error_rejected(#[case] duration: &str, #[case] seconds_step: bool) {
    let timezone = json!(null);
    let reset_time = json!(null);
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    let seconds = duration_in_seconds(
        duration,
        clock.calendar_now,
        clock.calendar_timezone,
        &DATABASE(),
    );
    assert_eq!(seconds.is_err(), seconds_step, "{duration}");
    assert!(
        !is_persistable_budget_duration(duration, raw(&timezone, &reset_time), clock, &DATABASE())
            .unwrap()
    );
}

#[rstest]
#[case::quantity("99999999999999999999d")]
#[case::reset_range("100000000d")]
fn overflow_rejected(#[case] duration: &str) {
    let timezone = json!(null);
    let reset_time = json!(null);
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    let failure = get_budget_reset_time(
        duration,
        raw(&timezone, &reset_time),
        clock.reset_now,
        &DATABASE(),
    )
    .unwrap_err();
    assert!(matches!(
        failure,
        BudgetResetError::Duration(
            DurationError::QuantityOverflow { .. } | DurationError::Calendar { .. }
        )
    ));
    assert!(
        !is_persistable_budget_duration(duration, raw(&timezone, &reset_time), clock, &DATABASE())
            .unwrap()
    );
}

#[rstest]
#[case::timezone(json!(1_i32), json!(null))]
#[case::reset_time(json!(null), json!(0_i32))]
fn unrelated_error_propagates(#[case] timezone: JsonValue, #[case] reset_time: JsonValue) {
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    let persist_error =
        is_persistable_budget_duration("1h", raw(&timezone, &reset_time), clock, &DATABASE())
            .unwrap_err();
    assert!(matches!(
        persist_error,
        BudgetResetError::Settings(
            BudgetResetSettingsError::TimezoneNotText | BudgetResetSettingsError::ResetTime(_)
        )
    ));
    let duration_failure =
        budget_duration_error(Some("1h"), raw(&timezone, &reset_time), clock, &DATABASE())
            .unwrap_err();
    assert!(matches!(duration_failure, BudgetResetError::Settings(_)));
}

#[test]
fn positive_prefix_unknown_reset_unit_accepted() {
    let timezone = json!(null);
    let reset_time = json!(null);
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    assert!(
        is_persistable_budget_duration("1hour", raw(&timezone, &reset_time), clock, &DATABASE())
            .unwrap()
    );
    assert_eq!(
        budget_duration_error(
            Some("1hour"),
            raw(&timezone, &reset_time),
            clock,
            &DATABASE()
        )
        .unwrap(),
        None
    );
    let reset = get_budget_reset_time(
        "1hour",
        raw(&timezone, &reset_time),
        clock.reset_now,
        &DATABASE(),
    )
    .unwrap();
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
    let timezone = json!(null);
    let reset_time = json!(null);
    let clock = clocks("2024-06-15T12:00:00Z", "2024-06-15T12:00:00Z", None);
    let message =
        budget_duration_error(Some("abc"), raw(&timezone, &reset_time), clock, &DATABASE())
            .unwrap()
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
