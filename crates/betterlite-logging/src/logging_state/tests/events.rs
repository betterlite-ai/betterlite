use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{build, encoded, init, moment, params, text};
use crate::logging_state::LoggingInit;
use crate::logging_state::events::{
    LoggingEvent, claim_async_success_log, has_run_logging, is_sync_litellm_request,
    set_response_timing_metrics, update_completion_start_time,
};

#[derive(Clone, Copy)]
enum AsyncFlag {
    None,
    Acompletion,
    Aresponses,
    Aembedding,
    AimageGeneration,
    Atranscription,
    AllmPassthroughRoute,
    AanthropicMessages,
    AgenerateContent,
    AgenerateContentStream,
    Arealtime,
    NonBoolean,
}

enum Scenario {
    ShouldRunBefore,
    Record(Option<bool>, LoggingEvent, LoggingEvent),
    ClaimTwice,
    CompletionStartTime,
    TimingReplaced,
    Sync(AsyncFlag),
}

fn async_flag(value: AsyncFlag) -> Option<&'static str> {
    match value {
        AsyncFlag::None | AsyncFlag::NonBoolean => None,
        AsyncFlag::Acompletion => Some("acompletion"),
        AsyncFlag::Aresponses => Some("aresponses"),
        AsyncFlag::Aembedding => Some("aembedding"),
        AsyncFlag::AimageGeneration => Some("aimage_generation"),
        AsyncFlag::Atranscription => Some("atranscription"),
        AsyncFlag::AllmPassthroughRoute => Some("allm_passthrough_route"),
        AsyncFlag::AanthropicMessages => Some("aanthropic_messages"),
        AsyncFlag::AgenerateContent => Some("agenerate_content"),
        AsyncFlag::AgenerateContentStream => Some("agenerate_content_stream"),
        AsyncFlag::Arealtime => Some("_arealtime"),
    }
}

fn should_run_outcome() -> String {
    let state = build(init());
    let events = [
        LoggingEvent::AsyncSuccess,
        LoggingEvent::SyncSuccess,
        LoggingEvent::AsyncFailure,
        LoggingEvent::SyncFailure,
    ];
    events
        .map(|event| {
            (state.model_call_details.get(event.key()) != Some(&Json::Bool(true))).to_string()
        })
        .join("/")
}

fn record_outcome(stream: Option<bool>, recorded: LoggingEvent, untouched: LoggingEvent) -> String {
    let mut state = build(LoggingInit { stream, ..init() });
    let before = state.model_call_details.get(recorded.key()) != Some(&Json::Bool(true));
    has_run_logging(&mut state, recorded);
    format!(
        "{}/{}/{}",
        before,
        state.model_call_details.get(recorded.key()) != Some(&Json::Bool(true)),
        state.model_call_details.get(untouched.key()) != Some(&Json::Bool(true))
    )
}

fn claim_outcome() -> String {
    let mut state = build(init());
    let first = claim_async_success_log(&mut state);
    let second = claim_async_success_log(&mut state);
    format!("{first}/{second}")
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

fn sync_outcome(flag: AsyncFlag) -> String {
    let values = match flag {
        AsyncFlag::NonBoolean => params(&[("acompletion", text("true"))]),
        AsyncFlag::None
        | AsyncFlag::Acompletion
        | AsyncFlag::Aresponses
        | AsyncFlag::Aembedding
        | AsyncFlag::AimageGeneration
        | AsyncFlag::Atranscription
        | AsyncFlag::AllmPassthroughRoute
        | AsyncFlag::AanthropicMessages
        | AsyncFlag::AgenerateContent
        | AsyncFlag::AgenerateContentStream
        | AsyncFlag::Arealtime => {
            async_flag(flag).map_or_else(IndexMap::new, |key| params(&[(key, Json::Bool(true))]))
        }
    };
    is_sync_litellm_request(&values).to_string()
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
        Scenario::Sync(flag) => sync_outcome(*flag),
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
    r#""2026-01-01T00:00:05Z"/"2026-01-01T00:00:05+00:00""#
)]
#[case::timing_metrics_are_replaced_not_merged(Scenario::TimingReplaced, r#"{"second":3.0}"#)]
#[case::a_request_without_async_flags_is_sync(Scenario::Sync(AsyncFlag::None), "true")]
#[case::acompletion_is_async(Scenario::Sync(AsyncFlag::Acompletion), "false")]
#[case::aresponses_is_async(Scenario::Sync(AsyncFlag::Aresponses), "false")]
#[case::aembedding_is_async(Scenario::Sync(AsyncFlag::Aembedding), "false")]
#[case::aimage_generation_is_async(Scenario::Sync(AsyncFlag::AimageGeneration), "false")]
#[case::atranscription_is_async(Scenario::Sync(AsyncFlag::Atranscription), "false")]
#[case::allm_passthrough_route_is_async(Scenario::Sync(AsyncFlag::AllmPassthroughRoute), "false")]
#[case::aanthropic_messages_is_async(Scenario::Sync(AsyncFlag::AanthropicMessages), "false")]
#[case::agenerate_content_is_async(Scenario::Sync(AsyncFlag::AgenerateContent), "false")]
#[case::agenerate_content_stream_is_async(
    Scenario::Sync(AsyncFlag::AgenerateContentStream),
    "false"
)]
#[case::arealtime_is_async(Scenario::Sync(AsyncFlag::Arealtime), "false")]
#[case::a_string_async_flag_does_not_count(Scenario::Sync(AsyncFlag::NonBoolean), "true")]
fn logging_state_behavior_tests_behavior(#[case] scenario: Scenario, #[case] expected: &str) {
    assert_eq!(outcome(&scenario), expected);
}
