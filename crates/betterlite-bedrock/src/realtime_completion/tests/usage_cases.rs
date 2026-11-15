use rstest::rstest;

use super::support::{EVENT_1, FixedIds, open_state, wire};
use crate::agentcore_stream::IdSource as _;
use crate::realtime_completion::error::CompletionError;
use crate::realtime_completion::records::DeltaType;
use crate::realtime_completion::usage::{
    leftover_usage_done_events, response_done_events, take_usage_delta,
};
use crate::realtime_session::RealtimeSession;
use crate::realtime_session::records::UsageEvent;

const FIRST_USAGE: &str = r#"{"totalInputTokens":100,"totalOutputTokens":50,"totalTokens":150,"details":{"total":{"input":{"speechTokens":60,"textTokens":40},"output":{"speechTokens":30,"textTokens":20}}}}"#;
const FIRST_DELTA: &str = r#"{"input_tokens":100,"output_tokens":50,"total_tokens":150,"input_token_details":{"audio_tokens":60,"text_tokens":40,"cached_tokens":0},"output_token_details":{"audio_tokens":30,"text_tokens":20}}"#;
const ZERO_DELTA: &str = r#"{"input_tokens":0,"output_tokens":0,"total_tokens":0,"input_token_details":{"audio_tokens":0,"text_tokens":0,"cached_tokens":0},"output_token_details":{"audio_tokens":0,"text_tokens":0}}"#;

fn parse_usage(text: &str) -> UsageEvent {
    serde_json::from_str(text).expect("a usage fixture")
}

pub(super) fn incremental_usage_delta() {
    let mut session = RealtimeSession::new(&FixedIds::sequence(3));
    session.cumulative_usage = parse_usage(FIRST_USAGE);
    let first = take_usage_delta(&mut session).expect("the first delta");
    assert_eq!(
        serde_json::to_string(&first).expect("usage serializes"),
        FIRST_DELTA
    );
    assert_eq!(session.reported_usage, session.cumulative_usage);

    let repeated = take_usage_delta(&mut session).expect("the repeated delta");
    assert_eq!(
        serde_json::to_string(&repeated).expect("usage serializes"),
        ZERO_DELTA
    );

    session.cumulative_usage = parse_usage(
        r#"{"totalInputTokens":90,"totalOutputTokens":70,"totalTokens":999,"details":{"total":{"input":{"speechTokens":55,"textTokens":45},"output":{"speechTokens":35,"textTokens":15}}}}"#,
    );
    let shrunk = take_usage_delta(&mut session).expect("the shrunk delta");
    assert_eq!(
        serde_json::to_string(&shrunk).expect("usage serializes"),
        r#"{"input_tokens":-10,"output_tokens":20,"total_tokens":849,"input_token_details":{"audio_tokens":-5,"text_tokens":5,"cached_tokens":0},"output_token_details":{"audio_tokens":5,"text_tokens":-5}}"#
    );
    assert_eq!(session.reported_usage, session.cumulative_usage);
}

#[rstest]
#[case::input_tokens(
    r#"{"totalInputTokens":9223372036854775807}"#,
    r#"{"totalInputTokens":-1}"#
)]
#[case::output_tokens(
    r#"{"totalOutputTokens":9223372036854775807}"#,
    r#"{"totalOutputTokens":-1}"#
)]
#[case::total_tokens(r#"{"totalTokens":9223372036854775807}"#, r#"{"totalTokens":-1}"#)]
#[case::input_speech(
    r#"{"details":{"total":{"input":{"speechTokens":9223372036854775807}}}}"#,
    r#"{"details":{"total":{"input":{"speechTokens":-1}}}}"#
)]
#[case::input_text(
    r#"{"details":{"total":{"input":{"textTokens":9223372036854775807}}}}"#,
    r#"{"details":{"total":{"input":{"textTokens":-1}}}}"#
)]
#[case::output_speech(
    r#"{"details":{"total":{"output":{"speechTokens":9223372036854775807}}}}"#,
    r#"{"details":{"total":{"output":{"speechTokens":-1}}}}"#
)]
#[case::output_text(
    r#"{"details":{"total":{"output":{"textTokens":9223372036854775807}}}}"#,
    r#"{"details":{"total":{"output":{"textTokens":-1}}}}"#
)]
fn usage_delta_overflow_is_an_error(#[case] latest: &str, #[case] prior: &str) {
    let mut session = RealtimeSession::new(&FixedIds::sequence(3));
    session.cumulative_usage = parse_usage(latest);
    session.reported_usage = parse_usage(prior);
    assert!(matches!(
        take_usage_delta(&mut session),
        Err(CompletionError::UsageOverflow)
    ));
}

pub(super) fn prompt_end_response_done() {
    let mut session = RealtimeSession::new(&FixedIds::sequence(3));
    session.cumulative_usage = parse_usage(FIRST_USAGE);
    let mut state = open_state(Some(DeltaType::Text), Some(&["kept"]));
    let events = response_done_events(&mut session, &mut state, &FixedIds::sequence(1))
        .expect("the response ends");
    assert_eq!(
        wire(&events),
        [format!(
            r#"{{"type":"response.done","event_id":"{EVENT_1}","response":{{"object":"realtime.response","id":"resp_1","status":"completed","output":[],"conversation_id":"conv_1","usage":{FIRST_DELTA}}}}}"#
        )]
    );
    assert_eq!(state.output_item_id, None);
    assert_eq!(state.response_id, None);
    assert_eq!(state.delta_type, None);
    assert_eq!(state.conversation_id.as_deref(), Some("conv_1"));
    assert_eq!(state.delta_chunks.as_ref().map(Vec::len), Some(1));
    assert_eq!(session.reported_usage, session.cumulative_usage);
}

#[rstest]
#[case::without_conversation(None, Some("resp_1"))]
#[case::without_response(Some("conv_1"), None)]
fn prompt_end_without_both_ids_keeps_usage(
    #[case] conversation: Option<&str>,
    #[case] response: Option<&str>,
) {
    let mut session = RealtimeSession::new(&FixedIds::sequence(3));
    session.cumulative_usage = parse_usage(FIRST_USAGE);
    let mut state = open_state(Some(DeltaType::Audio), Some(&["kept"]));
    state.conversation_id = conversation.map(str::to_owned);
    state.response_id = response.map(str::to_owned);
    let ids = FixedIds::sequence(1);
    let events = response_done_events(&mut session, &mut state, &ids).expect("the guard returns");
    assert!(events.is_empty());
    assert_eq!(state.response_id, None);
    assert_eq!(state.output_item_id, None);
    assert_eq!(state.delta_type, None);
    assert_eq!(state.conversation_id.as_deref(), conversation);
    assert_eq!(state.delta_chunks.as_ref().map(Vec::len), Some(1));
    assert_eq!(session.reported_usage, UsageEvent::default());
    assert_eq!(ids.next_id(), uuid::Uuid::from_u128(1));
}

pub(super) fn leftover_usage_done() {
    let mut session = RealtimeSession::new(&FixedIds::sequence(3));
    let ids = FixedIds::sequence(3);
    let none = leftover_usage_done_events(&mut session, &ids).expect("no leftover");
    assert!(none.is_empty());
    assert_eq!(ids.next_id(), uuid::Uuid::from_u128(1));

    session.cumulative_usage = parse_usage(FIRST_USAGE);
    let fresh_ids = FixedIds::sequence(3);
    let events = leftover_usage_done_events(&mut session, &fresh_ids).expect("a leftover event");
    assert_eq!(
        wire(&events),
        [format!(
            r#"{{"type":"response.done","event_id":"{EVENT_1}","response":{{"object":"realtime.response","id":"resp_00000000-0000-0000-0000-000000000002","status":"completed","conversation_id":"conv_00000000-0000-0000-0000-000000000003","usage":{FIRST_DELTA}}}}}"#
        )]
    );
    assert_eq!(session.reported_usage, session.cumulative_usage);
    let again = leftover_usage_done_events(&mut session, &FixedIds::sequence(3))
        .expect("the leftover is consumed");
    assert!(again.is_empty());
}

#[rstest]
#[case::input_tokens(r#"{"totalInputTokens":1}"#)]
#[case::output_tokens(r#"{"totalOutputTokens":1}"#)]
#[case::total_tokens(r#"{"totalTokens":1}"#)]
#[case::input_speech(r#"{"details":{"total":{"input":{"speechTokens":1}}}}"#)]
#[case::input_text(r#"{"details":{"total":{"input":{"textTokens":1}}}}"#)]
#[case::output_speech(r#"{"details":{"total":{"output":{"speechTokens":1}}}}"#)]
#[case::output_text(r#"{"details":{"total":{"output":{"textTokens":1}}}}"#)]
fn leftover_usage_reports_every_difference(#[case] cumulative: &str) {
    let mut session = RealtimeSession::new(&FixedIds::sequence(3));
    session.cumulative_usage = parse_usage(cumulative);
    let events =
        leftover_usage_done_events(&mut session, &FixedIds::sequence(3)).expect("a leftover event");
    assert_eq!(events.len(), 1);
    assert_eq!(session.reported_usage, session.cumulative_usage);
}
