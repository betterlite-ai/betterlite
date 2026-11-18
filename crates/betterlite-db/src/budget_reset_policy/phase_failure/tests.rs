#![cfg(test)]

use super::{
    EventMetadata, PhaseFailureEvent, PhaseFailureSink, PhaseTiming, RESET_BUDGET_JOB_SERVICE,
    report_phase_failure,
};
use crate::budget_reset_policy::tests::logging::{Logs, subscriber};
use betterlite_models::decisions::Json;
use chrono::{DateTime, TimeZone as _, Utc};
use serde_json::Number;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use tracing::subscriber::set_default;

#[derive(Clone, Default)]
struct RecordingSink(Arc<Mutex<Vec<PhaseFailureEvent>>>);

impl RecordingSink {
    fn events(&self) -> Vec<PhaseFailureEvent> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PhaseFailureSink for RecordingSink {
    fn emit(&self, event: PhaseFailureEvent) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

fn moment(hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, hour, minute, second)
        .unwrap()
}

const fn timing(start: DateTime<Utc>, end: DateTime<Utc>) -> PhaseTiming {
    PhaseTiming {
        start_time: start,
        end_time: end,
    }
}

fn metadata() -> EventMetadata {
    BTreeMap::from([
        ("num_keys_found", Json::Number(Number::from(3_u8))),
        ("reset_truncated", Json::Bool(false)),
    ])
}

#[test]
fn phase_failure_reports_the_service_and_call_type() {
    let sink = RecordingSink::default();
    report_phase_failure(
        &sink,
        "reset_budget_keys",
        "Failed to reset 1 keys: k1: boom",
        timing(moment(10, 0, 0), moment(10, 0, 5)),
        metadata(),
    );
    let events = sink.events();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.service, RESET_BUDGET_JOB_SERVICE);
    assert_eq!(event.service, "reset_budget_job");
    assert_eq!(event.call_type, "reset_budget_keys");
    assert_eq!(event.error, "Failed to reset 1 keys: k1: boom");
}

#[test]
fn phase_failure_carries_start_and_end_times() {
    let sink = RecordingSink::default();
    let start = moment(10, 0, 0);
    let end = moment(10, 0, 5);
    report_phase_failure(
        &sink,
        "reset_budget_keys",
        "boom",
        timing(start, end),
        metadata(),
    );
    let events = sink.events();
    let event = &events[0];
    assert_eq!(event.start_time, start);
    assert_eq!(event.end_time, end);
    assert_eq!(event.duration_seconds.to_bits(), 5.0_f64.to_bits());
}

#[test]
fn phase_failure_carries_event_metadata() {
    let sink = RecordingSink::default();
    report_phase_failure(
        &sink,
        "reset_budget_users",
        "boom",
        timing(moment(10, 0, 0), moment(10, 0, 1)),
        metadata(),
    );
    let events = sink.events();
    let event = &events[0];
    assert_eq!(event.event_metadata, metadata());
    assert_eq!(
        event.event_metadata.get("num_keys_found"),
        Some(&Json::Number(Number::from(3_u8)))
    );
}

#[test]
fn phase_failure_backwards_clock_records_no_duration() {
    let sink = RecordingSink::default();
    report_phase_failure(
        &sink,
        "reset_budget_keys",
        "boom",
        timing(moment(10, 0, 5), moment(10, 0, 0)),
        metadata(),
    );
    let events = sink.events();
    let event = &events[0];
    assert_eq!(event.duration_seconds.to_bits(), 0.0_f64.to_bits());
}

#[test]
fn phase_failure_logs_the_call_type_and_error() {
    let logs = Logs::default();
    let sink = RecordingSink::default();
    let guard = set_default(subscriber(logs.clone()));
    report_phase_failure(
        &sink,
        "reset_budget_teams",
        "Failed to reset 2 teams",
        timing(moment(10, 0, 0), moment(10, 0, 1)),
        metadata(),
    );
    drop(guard);
    let text = logs.text();
    assert!(text.contains("Reset budget phase failed"));
    assert!(text.contains("reset_budget_teams"));
    assert!(text.contains("Failed to reset 2 teams"));
    assert_eq!(sink.events().len(), 1);
}
