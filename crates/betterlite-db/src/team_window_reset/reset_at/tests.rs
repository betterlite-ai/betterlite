#![cfg(test)]

use chrono::TimeDelta;

use crate::budget_permission_rows::JsonData;
use crate::team_window_reset::error::WindowResetError;
use crate::team_window_reset::reset_at::advance_window_reset_at;
use crate::team_window_reset::tests::support::{TestSchedule, moment, window_json, window_text};

const PAST_TEXT: &str = "2025-12-30T00:00:00+00:00";

#[test]
fn next_reset_at_is_written_as_python_isoformat() {
    let schedule = TestSchedule::fixed(moment(3600));
    let mut window = window_json("1h", None, Some(PAST_TEXT));
    let next = advance_window_reset_at("k1", &mut window, "1h", &schedule).unwrap();
    assert_eq!(next, moment(3600));
    assert_eq!(
        window_text(&window, "reset_at"),
        Some("2026-01-01T01:00:00+00:00".to_owned())
    );
    assert_eq!(schedule.requested(), vec!["1h".to_owned()]);
}

#[test]
fn next_reset_at_keeps_the_microseconds_python_renders() {
    let next = moment(3600)
        .checked_add_signed(TimeDelta::microseconds(123_456))
        .unwrap();
    let schedule = TestSchedule::fixed(next);
    let mut window = window_json("2h", None, Some(PAST_TEXT));
    advance_window_reset_at("k1", &mut window, "2h", &schedule).unwrap();
    assert_eq!(
        window_text(&window, "reset_at"),
        Some("2026-01-01T01:00:00.123456+00:00".to_owned())
    );
    assert_eq!(schedule.requested(), vec!["2h".to_owned()]);
}

#[test]
fn next_reset_at_is_added_to_a_window_that_had_none() {
    let schedule = TestSchedule::fixed(moment(3600));
    let mut window = window_json("1h", None, None);
    advance_window_reset_at("k1", &mut window, "1h", &schedule).unwrap();
    assert_eq!(
        window_text(&window, "reset_at"),
        Some("2026-01-01T01:00:00+00:00".to_owned())
    );
}

#[test]
fn a_refused_reset_time_leaves_the_window_untouched() {
    let schedule = TestSchedule::failing_for(moment(3600), "1h", "no such unit");
    let mut window = window_json("1h", None, Some(PAST_TEXT));
    let error = advance_window_reset_at("k1", &mut window, "1h", &schedule).unwrap_err();
    assert!(matches!(error, WindowResetError::Schedule { .. }));
    assert_eq!(window_text(&window, "reset_at"), Some(PAST_TEXT.to_owned()));
}

#[test]
fn a_window_that_is_not_an_object_cannot_take_a_reset_time() {
    let schedule = TestSchedule::fixed(moment(3600));
    let mut window = JsonData::Null;
    let error = advance_window_reset_at("k1", &mut window, "1h", &schedule).unwrap_err();
    assert!(matches!(error, WindowResetError::Json(_)));
}
