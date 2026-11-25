use chrono::{DateTime, Datelike as _, TimeZone as _, Timelike as _, Utc};
use rstest::rstest;
use secrecy::SecretString;

use super::super::dates::{DateRangeError, SpendReportDates, spend_report_dates};
use super::super::query::decode_scoped_report_query;
use super::super::scope::{ScopeCaller, ScopeError, resolve_scope};

fn midnight(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
}

#[test]
fn dates_are_utc_midnights_and_the_end_day_is_kept() {
    let dates = spend_report_dates(Some("2024-01-01"), Some("2024-01-31")).unwrap();
    assert_eq!(dates.start, midnight(2024, 1, 1));
    assert_eq!(dates.end, midnight(2024, 1, 31));
    assert_eq!(dates.start.hour(), 0);
    assert_eq!(dates.start.day(), 1);
}

#[rstest]
#[case::one_digit_month_and_day("2024-1-1")]
#[case::one_digit_month("2024-1-31")]
#[case::one_digit_day("2024-01-1")]
#[case::space_before_single_digit_day("2024-1- 9")]
#[case::zero_padded_day("2024-1-09")]
#[case::first_year("0001-01-01")]
#[case::last_year("9999-12-31")]
fn cpython_date_forms_are_accepted(#[case] value: &str) {
    let dates = spend_report_dates(Some(value), Some(value)).unwrap();
    assert_eq!(dates.start, dates.end);
}

#[rstest]
#[case::empty("")]
#[case::space_before_month("2024- 1-01")]
#[case::two_spaces_before_day("2024-1-  1")]
#[case::two_digits_after_space("2024-1- 09")]
#[case::three_digit_day("2024-1-001")]
#[case::zero_year("0000-01-01")]
#[case::two_digit_year("24-01-01")]
#[case::signed_year("+024-01-01")]
#[case::signed_month("2024-+1-01")]
#[case::three_digit_month("2024-001-01")]
#[case::month_thirteen("2024-13-01")]
#[case::month_zero("2024-00-01")]
#[case::day_zero("2024-01-00")]
#[case::february_thirty("2024-02-30")]
#[case::leading_space(" 2024-01-01")]
#[case::trailing_space("2024-01-01 ")]
#[case::trailing_text("2024-01-01x")]
#[case::no_dashes("20240101")]
fn malformed_dates_are_a_format_error(#[case] value: &str) {
    assert_eq!(
        spend_report_dates(Some(value), Some("2024-01-01")),
        Err(DateRangeError::Format)
    );
}

#[test]
fn a_missing_bound_wins_over_a_malformed_one() {
    assert_eq!(
        spend_report_dates(None, Some("not-a-date")),
        Err(DateRangeError::Missing)
    );
    assert_eq!(
        spend_report_dates(Some("not-a-date"), None),
        Err(DateRangeError::Missing)
    );
    assert_eq!(spend_report_dates(None, None), Err(DateRangeError::Missing));
}

#[test]
fn a_reversed_range_is_rejected_before_its_length() {
    assert_eq!(
        spend_report_dates(Some("2024-01-02"), Some("2024-01-01")),
        Err(DateRangeError::Reversed)
    );
    assert_eq!(
        spend_report_dates(Some("2024-01-01"), Some("2024-01-01")),
        Ok(SpendReportDates {
            start: midnight(2024, 1, 1),
            end: midnight(2024, 1, 1),
        })
    );
}

#[test]
fn exactly_366_days_is_accepted_and_367_is_rejected() {
    spend_report_dates(Some("2024-01-01"), Some("2025-01-01")).unwrap();
    assert_eq!(
        spend_report_dates(Some("2024-01-01"), Some("2025-01-02")),
        Err(DateRangeError::TooLarge)
    );
}

#[test]
fn date_errors_render_litellms_messages() {
    assert_eq!(
        DateRangeError::Missing.to_string(),
        "Please provide start_date and end_date"
    );
    assert_eq!(
        DateRangeError::Format.to_string(),
        "start_date and end_date must be in YYYY-MM-DD format"
    );
    assert_eq!(
        DateRangeError::Reversed.to_string(),
        "start_date must be on or before end_date"
    );
    assert_eq!(
        DateRangeError::TooLarge.to_string(),
        "Date range too large; maximum is 366 days"
    );
}

#[rstest]
#[case::own_value(Some("mine"), Some("mine"), false, Ok("mine"))]
#[case::admin_any_value(Some("other"), None, true, Ok("other"))]
#[case::empty_request_falls_back(Some(""), Some("mine"), false, Ok("mine"))]
#[case::empty_caller_is_retained(Some(""), Some(""), false, Ok(""))]
#[case::absent_request_falls_back(None, Some("mine"), false, Ok("mine"))]
#[case::foreign_value(
    Some("other"),
    Some("mine"),
    false,
    Err(ScopeError::Forbidden("api_key"))
)]
#[case::foreign_value_without_caller(
    Some("other"),
    None,
    false,
    Err(ScopeError::Forbidden("api_key"))
)]
#[case::absent_everything(None, None, false, Err(ScopeError::Missing("api_key")))]
fn scope_resolution_clamps_non_admins(
    #[case] requested: Option<&str>,
    #[case] caller_value: Option<&str>,
    #[case] admin_view: bool,
    #[case] expected: Result<&str, ScopeError>,
) {
    assert_eq!(
        resolve_scope(
            requested,
            ScopeCaller::Value(caller_value),
            admin_view,
            "api_key"
        ),
        expected.map(str::to_owned)
    );
}

#[test]
fn key_scopes_match_the_logged_key_only() {
    let own = SecretString::from("sk-own");
    assert_eq!(
        resolve_scope(
            Some("sk-own"),
            ScopeCaller::Key(Some(&own)),
            false,
            "api_key"
        ),
        Ok("sk-own".to_owned())
    );
    assert_eq!(
        resolve_scope(
            Some("sk-other"),
            ScopeCaller::Key(Some(&own)),
            false,
            "api_key"
        ),
        Err(ScopeError::Forbidden("api_key"))
    );
    assert_eq!(
        resolve_scope(None, ScopeCaller::Key(Some(&own)), false, "api_key"),
        Ok("sk-own".to_owned())
    );
    assert_eq!(
        resolve_scope(Some("sk-any"), ScopeCaller::Key(None), true, "api_key"),
        Ok("sk-any".to_owned())
    );
    assert_eq!(
        resolve_scope(None, ScopeCaller::Key(None), false, "api_key"),
        Err(ScopeError::Missing("api_key"))
    );
}

#[test]
fn query_parsing_decodes_every_report_parameter() {
    let query = decode_scoped_report_query(
        b"start_date=2024-01-01&end_date=2024-02-01&api_key=sk-a&internal_user_id=u+1&team_id=t1&organization_id=o1",
    );
    assert_eq!(query.start_date.as_deref(), Some("2024-01-01"));
    assert_eq!(query.end_date.as_deref(), Some("2024-02-01"));
    assert_eq!(query.api_key.as_deref(), Some("sk-a"));
    assert_eq!(query.internal_user_id.as_deref(), Some("u 1"));
    assert_eq!(query.team_id.as_deref(), Some("t1"));
    assert_eq!(query.organization_id.as_deref(), Some("o1"));
}

#[test]
fn a_repeated_parameter_keeps_its_last_value() {
    let query = decode_scoped_report_query(b"api_key=first&api_key=second&team_id=");
    assert_eq!(query.api_key.as_deref(), Some("second"));
    assert_eq!(query.team_id.as_deref(), Some(""));
    assert_eq!(query.end_date, None);
}

#[test]
fn percent_encoded_values_are_decoded() {
    let query = decode_scoped_report_query(b"internal_user_id=%C3%A9t%C3%A9");
    assert_eq!(query.internal_user_id.as_deref(), Some("\u{e9}t\u{e9}"));
}
