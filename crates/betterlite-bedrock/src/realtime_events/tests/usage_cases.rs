use betterlite_core::api::realtime_contracts::events::RealtimeResponseTypedDict;

use super::support;
use crate::realtime_events::decode::json_dict;
use crate::realtime_events::error::EventError;

const USAGE_EVENT: &str = r#"{"event":{"usageEvent":{"totalInputTokens":10,"totalOutputTokens":4,"totalTokens":14,"details":{"total":{"input":{"speechTokens":2,"textTokens":8},"output":{"speechTokens":1,"textTokens":3}}}}}}"#;
const USAGE_JSON: &str = r#"{"input_tokens":10,"output_tokens":4,"total_tokens":14,"input_token_details":{"audio_tokens":2,"text_tokens":8,"cached_tokens":0},"output_token_details":{"audio_tokens":1,"text_tokens":3}}"#;
const ZERO_USAGE_JSON: &str = r#"{"input_tokens":0,"output_tokens":0,"total_tokens":0,"input_token_details":{"audio_tokens":0,"text_tokens":0,"cached_tokens":0},"output_token_details":{"audio_tokens":0,"text_tokens":0}}"#;
const CONTENT_START: &str = r#"{"event":{"contentStart":{"role":"ASSISTANT","type":"TEXT"}}}"#;

// Every nested record must remain an object; decoding failures cannot replace earlier usage.
#[rstest::rstest]
#[case::details(r#"{"details":[]}"#)]
#[case::total(r#"{"details":{"total":[]}}"#)]
#[case::input(r#"{"details":{"total":{"input":[2,8]}}}"#)]
#[case::output(r#"{"details":{"total":{"output":[1,3]}}}"#)]
#[case::null(r#"{"details":{"total":null}}"#)]
fn nested_usage_records(#[case] payload: &str) {
    assert_usage_rejected(payload);
}

pub(super) fn assert_usage_rejected(payload: &str) {
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let input = support::input_with_ids("item", "resp", "conv");
    support::transform(&mut session, &ids, USAGE_EVENT, &input).expect("initial usage");
    let previous = session.cumulative_usage.clone();
    let message = format!(r#"{{"event":{{"usageEvent":{payload}}}}}"#);
    assert!(matches!(
        support::transform(&mut session, &ids, &message, &input),
        Err(EventError::Payload {
            field: "usageEvent",
            ..
        })
    ));
    assert_eq!(session.cumulative_usage, previous);
}

// A payload of the wrong shape is an error and leaves the cumulative counters as they were
pub(super) fn usage_array_rejected() {
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let mut input = support::input();
    let started =
        support::transform(&mut session, &ids, CONTENT_START, &input).expect("a content start");
    input = support::next_input(&started);
    support::transform(&mut session, &ids, USAGE_EVENT, &input).expect("a usage event");
    let error = support::transform(&mut session, &ids, r#"{"event":{"usageEvent":[]}}"#, &input)
        .expect_err("an array usage event");
    assert!(matches!(
        error,
        EventError::Payload {
            field: "usageEvent",
            ..
        }
    ));
    let ended = support::transform(&mut session, &ids, r#"{"event":{"promptEnd":{}}}"#, &input)
        .expect("a prompt end");
    assert_usage(&ended, USAGE_JSON);
}

pub(super) fn usage_delta() {
    let ids = support::CountingIds::new();
    let mut session = support::new_session();
    let mut input = support::input();
    let started =
        support::transform(&mut session, &ids, CONTENT_START, &input).expect("a content start");
    input = support::next_input(&started);
    support::transform(&mut session, &ids, USAGE_EVENT, &input).expect("a usage event");
    let ended = support::transform(&mut session, &ids, r#"{"event":{"promptEnd":{}}}"#, &input)
        .expect("a prompt end");
    assert_usage(&ended, USAGE_JSON);
    input = support::next_input(&ended);
    let restarted =
        support::transform(&mut session, &ids, CONTENT_START, &input).expect("a content start");
    input = support::next_input(&restarted);
    let ended_again =
        support::transform(&mut session, &ids, r#"{"event":{"promptEnd":{}}}"#, &input)
            .expect("a prompt end");
    assert_usage(&ended_again, ZERO_USAGE_JSON);
}

fn assert_usage(response: &RealtimeResponseTypedDict, expected: &str) {
    let events = support::events(response);
    assert_eq!(events.len(), 1);
    let done = json_dict(&events[0]).expect("a response.done object");
    let object = done
        .get("response")
        .and_then(json_dict)
        .expect("a response object");
    assert_eq!(object.get("usage"), Some(&support::json(expected)));
}
