#![cfg(test)]

use chrono::{DateTime, NaiveDate, Utc};

use crate::reload_schedule::ReloadSchedule;
use crate::reload_schedule::status_formatting::reload_schedule_status;

fn at(hour: u32) -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(2026, 3, 4)
        .unwrap()
        .and_hms_opt(hour, 6, 7)
        .unwrap()
        .and_utc()
}

#[test]
fn status_formatting_reports_no_schedule() {
    let status = reload_schedule_status(None);
    assert!(!status.scheduled);
    assert_eq!(status.interval_hours, None);
    assert_eq!(status.last_run, None);
    assert_eq!(status.next_run, None);
}

#[test]
fn status_formatting_writes_the_typed_dict_fields() {
    let status = reload_schedule_status(None);
    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        r#"{"scheduled":false,"interval_hours":null,"last_run":null,"next_run":null}"#
    );
}

#[test]
fn status_formatting_writes_iso_times_for_a_schedule() {
    let schedule = ReloadSchedule {
        interval_hours: Some(6),
        reload_revision: 2,
        last_run_at: Some(at(5)),
    };
    let status = reload_schedule_status(Some(&schedule));
    assert!(status.scheduled);
    assert_eq!(status.interval_hours, Some(6));
    assert_eq!(
        status.last_run.as_deref(),
        Some("2026-03-04T05:06:07+00:00")
    );
    assert_eq!(
        status.next_run.as_deref(),
        Some("2026-03-04T11:06:07+00:00")
    );
}

#[test]
fn status_formatting_marks_a_cleared_interval_unscheduled() {
    let schedule = ReloadSchedule {
        interval_hours: None,
        reload_revision: 2,
        last_run_at: Some(at(5)),
    };
    let status = reload_schedule_status(Some(&schedule));
    assert!(!status.scheduled);
    assert_eq!(status.interval_hours, None);
    assert_eq!(
        status.last_run.as_deref(),
        Some("2026-03-04T05:06:07+00:00")
    );
    assert_eq!(status.next_run, None);
}

#[test]
fn status_formatting_leaves_a_schedule_without_a_run_unrun() {
    let schedule = ReloadSchedule {
        interval_hours: Some(6),
        reload_revision: 0,
        last_run_at: None,
    };
    let status = reload_schedule_status(Some(&schedule));
    assert!(status.scheduled);
    assert_eq!(status.last_run, None);
    assert_eq!(status.next_run, None);
}
