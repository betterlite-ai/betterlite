use indexmap::IndexMap;
use rstest::rstest;

use super::support::{build, encoded, init, moment};
use crate::logging_state::LoggingInit;
use crate::logging_state::events::{
    LoggingEvent, claim_async_success_log, has_run_logging, set_response_timing_metrics,
    should_run_logging, update_completion_start_time,
};

enum Scenario {
    ShouldRunBefore,
    Record(Option<bool>, LoggingEvent, LoggingEvent),
    ClaimTwice,
    CompletionStartTime,
    TimingReplaced,
}

fn should_run_outcome() -> String {
    let state = build(init());
    format!(
        "{}/{}/{}/{}",
        should_run_logging(&state, LoggingEvent::AsyncSuccess),
        should_run_logging(&state, LoggingEvent::SyncSuccess),
        should_run_logging(&state, LoggingEvent::AsyncFailure),
        should_run_logging(&state, LoggingEvent::SyncFailure)
    )
}

fn record_outcome(stream: Option<bool>, recorded: LoggingEvent, untouched: LoggingEvent) -> String {
    let mut state = build(LoggingInit { stream, ..init() });
    let before = should_run_logging(&state, recorded);
    has_run_logging(&mut state, recorded);
    format!(
        "{}/{}/{}",
        before,
        should_run_logging(&state, recorded),
        should_run_logging(&state, untouched)
    )
}

fn claim_outcome() -> String {
    let mut state = build(init());
    format!(
        "{}/{}",
        claim_async_success_log(&mut state),
        claim_async_success_log(&mut state)
    )
}

fn completion_outcome() -> String {
    let mut state = build(init());
    update_completion_start_time(&mut state, moment("2026-01-01T00:00:05+00:00"));
    format!(
        "{}/{}",
        encoded(&state.completion_start_time),
        encoded(
            &state
                .model_call_details
                .get("completion_start_time")
                .cloned()
        )
    )
}

fn timing_outcome() -> String {
    let mut state = build(init());
    let mut first = IndexMap::new();
    first.insert("first".to_owned(), 1.0_f64);
    first.insert("second".to_owned(), 2.0_f64);
    set_response_timing_metrics(&mut state, first);
    let mut second = IndexMap::new();
    second.insert("second".to_owned(), 3.0_f64);
    set_response_timing_metrics(&mut state, second);
    encoded(&state.response_timing_metrics)
}

fn outcome(scenario: &Scenario) -> String {
    match scenario {
        Scenario::ShouldRunBefore => should_run_outcome(),
        Scenario::Record(stream, recorded, untouched) => {
            record_outcome(*stream, *recorded, *untouched)
        }
        Scenario::ClaimTwice => claim_outcome(),
        Scenario::CompletionStartTime => completion_outcome(),
        Scenario::TimingReplaced => timing_outcome(),
    }
}

#[rstest]
#[case::every_event_runs_before_it_is_logged(Scenario::ShouldRunBefore, "true/true/true/true")]
#[case::recording_an_event_suppresses_only_that_event(
    Scenario::Record(Some(false), LoggingEvent::SyncSuccess, LoggingEvent::AsyncSuccess),
    "true/false/true"
)]
#[case::a_stream_never_records_a_log(
    Scenario::Record(Some(true), LoggingEvent::SyncSuccess, LoggingEvent::AsyncFailure),
    "true/true/true"
)]
#[case::an_unknown_stream_value_records(
    Scenario::Record(None, LoggingEvent::AsyncFailure, LoggingEvent::SyncFailure),
    "true/false/true"
)]
#[case::the_first_claim_wins(Scenario::ClaimTwice, "true/false")]
#[case::completion_start_time_is_stored_twice(
    Scenario::CompletionStartTime,
    r#""2026-01-01T00:00:05+00:00"/"2026-01-01T00:00:05+00:00""#
)]
#[case::timing_metrics_are_replaced_not_merged(Scenario::TimingReplaced, r#"{"second":3.0}"#)]
fn logging_state_behavior_tests_behavior(#[case] scenario: Scenario, #[case] expected: &str) {
    assert_eq!(outcome(&scenario), expected);
}
