#![cfg(test)]

use std::collections::BTreeMap;

use crate::budget_permission_rows::JsonData;
use crate::reload_schedule::parse_schedule::{
    ParseScheduleError, ReloadConfigRow, parse_reload_schedule,
};

fn config_row(param_value: JsonData) -> ReloadConfigRow {
    ReloadConfigRow {
        param_value,
        last_run_at: None,
        reload_revision: None,
    }
}

#[test]
fn parse_schedule_reads_a_serialized_json_string() {
    let schedule = parse_reload_schedule(&config_row(JsonData::String(
        r#"{"interval_hours": 8}"#.to_owned(),
    )))
    .unwrap();
    assert_eq!(schedule.interval_hours, Some(8));
}

#[test]
fn parse_schedule_reads_a_decoded_object() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "interval_hours".to_owned(),
        JsonData::Number(serde_json::Number::from(6_i32)),
    );
    let schedule = parse_reload_schedule(&config_row(JsonData::Object(fields))).unwrap();
    assert_eq!(schedule.interval_hours, Some(6));
}

#[test]
fn parse_schedule_keeps_a_null_interval_as_no_schedule() {
    let schedule = parse_reload_schedule(&config_row(JsonData::String(
        r#"{"interval_hours": null}"#.to_owned(),
    )))
    .unwrap();
    assert_eq!(schedule.interval_hours, None);
    assert_eq!(schedule.reload_revision, 0);
    assert_eq!(schedule.last_run_at, None);
}

#[test]
fn parse_schedule_defaults_a_missing_revision_to_zero() {
    let schedule = parse_reload_schedule(&config_row(JsonData::Object(BTreeMap::new()))).unwrap();
    assert_eq!(schedule.reload_revision, 0);
}

#[test]
fn parse_schedule_keeps_the_stored_revision() {
    let row = ReloadConfigRow {
        param_value: JsonData::String(r#"{"interval_hours": 1}"#.to_owned()),
        last_run_at: None,
        reload_revision: Some(7),
    };
    assert_eq!(parse_reload_schedule(&row).unwrap().reload_revision, 7);
}

#[test]
fn parse_schedule_rejects_a_revision_past_u32() {
    let row = ReloadConfigRow {
        param_value: JsonData::Null,
        last_run_at: None,
        reload_revision: Some(i64::from(u32::MAX) + 1),
    };
    let error = parse_reload_schedule(&row).unwrap_err();
    assert!(matches!(
        error,
        ParseScheduleError::Revision { value } if value == i64::from(u32::MAX) + 1
    ));
}

#[test]
fn parse_schedule_rejects_a_negative_revision() {
    let row = ReloadConfigRow {
        param_value: JsonData::Null,
        last_run_at: None,
        reload_revision: Some(-1),
    };
    let error = parse_reload_schedule(&row).unwrap_err();
    assert!(matches!(error, ParseScheduleError::Revision { value: -1 }));
}
