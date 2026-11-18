#![cfg(test)]

use chrono::{NaiveDate, NaiveDateTime};

use crate::budget_permission_rows::JsonData;
use crate::reload_schedule::parse_schedule::{ReloadConfigRow, parse_reload_schedule};

fn naive() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 3, 4)
        .unwrap()
        .and_hms_opt(5, 6, 7)
        .unwrap()
}

#[test]
fn timezone_handling_attaches_utc_to_a_naive_timestamp() {
    let row = ReloadConfigRow {
        param_value: JsonData::String(r#"{"interval_hours": 6}"#.to_owned()),
        last_run_at: Some(naive()),
        reload_revision: Some(2),
    };
    let schedule = parse_reload_schedule(&row);
    assert_eq!(schedule.last_run_at, Some(naive().and_utc()));
    assert_eq!(
        schedule.last_run_at.unwrap().to_rfc3339(),
        "2026-03-04T05:06:07+00:00"
    );
}

#[test]
fn timezone_handling_leaves_a_missing_timestamp_absent() {
    let row = ReloadConfigRow {
        param_value: JsonData::Null,
        last_run_at: None,
        reload_revision: Some(0),
    };
    assert_eq!(parse_reload_schedule(&row).last_run_at, None);
}
