use chrono::NaiveDate;
use rstest::rstest;

use crate::user_agent_analytics::error::AnalyticsError;
use crate::user_agent_analytics::rows::ActiveUsersRow;
use crate::user_agent_analytics::tests::support::at;
use crate::user_agent_analytics::tests::support::fixed_clock;
use crate::user_agent_analytics::tests::support::now;
use crate::user_agent_analytics::window::MAX_DAYS;
use crate::user_agent_analytics::window::WAU_SPAN_DAYS;
use crate::user_agent_analytics::window::Window;
use crate::user_agent_analytics::window::weekly_result;

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("a date")
}

fn window() -> Window {
    Window::ending_tomorrow(fixed_clock(now()).as_ref(), WAU_SPAN_DAYS).expect("a window")
}

fn row(week_offset: i64) -> ActiveUsersRow {
    ActiveUsersRow {
        tag: "User-Agent:alpha".to_owned(),
        active_users: 4,
        date: week_offset.to_string(),
    }
}

#[test]
fn the_daily_window_starts_six_days_before_utc_tomorrow() {
    let window = Window::ending_tomorrow(fixed_clock(at(2026, 1, 10, 23, 59)).as_ref(), MAX_DAYS)
        .expect("a window");
    assert_eq!(window.start_date, "2026-01-04");
    assert_eq!(window.end_date, "2026-01-11");
    assert_eq!(window.end_day, date(2026, 1, 11));
}

#[test]
fn the_weekly_window_starts_forty_eight_days_before_utc_tomorrow() {
    let window = window();
    assert_eq!(window.start_date, "2025-11-24");
    assert_eq!(window.end_date, "2026-01-11");
    assert_eq!(window.end_day, date(2026, 1, 11));
}

#[test]
fn a_clock_past_the_calendar_reports_a_window_error() {
    let last_day = NaiveDate::MAX
        .and_hms_opt(0, 0, 0)
        .expect("the last midnight")
        .and_utc();
    let error = Window::ending_tomorrow(fixed_clock(last_day).as_ref(), MAX_DAYS)
        .expect_err("the calendar ends");
    assert!(matches!(error, AnalyticsError::Window));
}

#[test]
fn a_window_that_starts_before_the_calendar_reports_a_window_error() {
    let first_day = NaiveDate::MIN
        .and_hms_opt(0, 0, 0)
        .expect("the first midnight")
        .and_utc();
    let error = Window::ending_tomorrow(fixed_clock(first_day).as_ref(), WAU_SPAN_DAYS)
        .expect_err("the calendar starts");
    assert!(matches!(error, AnalyticsError::Window));
}

#[rstest]
#[case::this_week(0, "Week 7 (Jan 05)", "2026-01-05", "2026-01-11")]
#[case::last_week(1, "Week 6 (Dec 29)", "2025-12-29", "2026-01-04")]
#[case::first_week(6, "Week 1 (Nov 24)", "2025-11-24", "2025-11-30")]
fn a_week_row_becomes_its_labelled_period(
    #[case] offset: i64,
    #[case] label: &str,
    #[case] start: &str,
    #[case] end: &str,
) {
    let point = weekly_result(&window(), row(offset)).expect("a week point");
    assert_eq!(point.tag, "User-Agent:alpha");
    assert_eq!(point.active_users, 4);
    assert_eq!(point.date, label);
    assert_eq!(point.period_start.as_deref(), Some(start));
    assert_eq!(point.period_end.as_deref(), Some(end));
}

#[rstest]
#[case::negative(-1)]
#[case::past_the_seventh_week(8)]
#[case::beyond_the_calendar(i64::MAX)]
fn an_impossible_week_offset_reports_an_offset_error(#[case] offset: i64) {
    let error = weekly_result(&window(), row(offset)).expect_err("the offset is impossible");
    assert!(matches!(error, AnalyticsError::Offset { .. }));
}

#[test]
fn a_week_bucket_that_is_not_an_offset_reports_a_bucket_error() {
    let mut week_row = row(0);
    week_row.date = "not-a-week".to_owned();
    let error = weekly_result(&window(), week_row).expect_err("the bucket is impossible");
    assert!(matches!(error, AnalyticsError::Bucket { bucket } if bucket == "not-a-week"));
}
