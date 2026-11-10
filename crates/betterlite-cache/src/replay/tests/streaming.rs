#![cfg(test)]

use std::time::Duration;

use rstest::rstest;

use crate::replay::{CacheHit, ReplayRequest, Replayed, ResponseKind, replay_cached_result};
use crate::types::call_type::CachingSupportedCallType;

use super::shared::{
    Fake, WrapRecord, chat_completion_dict, convert, converted_stream_request, dict_of,
    non_stream_request, stream_request, text_completion_dict,
};

fn expected_wrap(asynchronous: bool) -> WrapRecord {
    WrapRecord {
        asynchronous,
        model: "gpt-4o".to_owned(),
        custom_llm_provider: "cached_response".to_owned(),
    }
}

#[test]
fn streaming_stream_flag_wraps_the_cached_dict() {
    let fake = Fake::new();
    let outcome = convert(
        chat_completion_dict(),
        CachingSupportedCallType::Completion,
        &stream_request(),
        &fake,
    )
    .unwrap();
    assert!(matches!(outcome, Replayed::Stream(_)));
    let calls = fake.calls();
    assert_eq!(calls.wraps, vec![expected_wrap(false)]);
    assert_eq!(calls.wrapper_dict, Some(dict_of(chat_completion_dict())));
    assert!(calls.model_kinds.is_empty());
}

#[rstest]
#[case::completion(CachingSupportedCallType::Completion, false, false)]
#[case::acompletion(CachingSupportedCallType::Acompletion, true, false)]
#[case::text_completion(CachingSupportedCallType::TextCompletion, false, true)]
#[case::atext_completion(CachingSupportedCallType::AtextCompletion, true, true)]
fn streaming_the_converter_follows_the_call_type(
    #[case] call_type: CachingSupportedCallType,
    #[case] asynchronous: bool,
    #[case] text: bool,
) {
    let fake = Fake::new();
    let cached = if text {
        text_completion_dict()
    } else {
        chat_completion_dict()
    };
    let wrapped = dict_of(cached.clone());
    let outcome = convert(cached, call_type, &stream_request(), &fake).unwrap();
    assert!(matches!(outcome, Replayed::Stream(_)));
    let calls = fake.calls();
    assert_eq!(calls.wraps, vec![expected_wrap(asynchronous)]);
    assert_eq!(calls.wrapper_dict, Some(wrapped));
    assert!(calls.model_kinds.is_empty());
}

#[test]
fn streaming_converted_stream_request_replays_at_zero_depth() {
    let fake = Fake::new();
    let outcome = convert(
        chat_completion_dict(),
        CachingSupportedCallType::Completion,
        &converted_stream_request(0),
        &fake,
    )
    .unwrap();
    assert!(matches!(outcome, Replayed::Stream(_)));
    assert_eq!(fake.calls().wraps.len(), 1);
}

#[test]
fn streaming_converted_stream_request_inside_an_agentic_loop_is_not_replayed() {
    let fake = Fake::new();
    let outcome = convert(
        chat_completion_dict(),
        CachingSupportedCallType::Completion,
        &converted_stream_request(1),
        &fake,
    )
    .unwrap();
    assert!(matches!(outcome, Replayed::Response(ResponseKind::Chat, _)));
    assert!(fake.calls().wraps.is_empty());
}

#[test]
fn streaming_stream_flag_wins_inside_an_agentic_loop() {
    let fake = Fake::new();
    let request = ReplayRequest {
        stream: true,
        converted_stream_requested: false,
        agentic_loop_depth: 2,
        model: "gpt-4o",
        custom_llm_provider: None,
        elapsed: Duration::from_millis(7),
    };
    let outcome = convert(
        chat_completion_dict(),
        CachingSupportedCallType::Completion,
        &request,
        &fake,
    )
    .unwrap();
    assert!(matches!(outcome, Replayed::Stream(_)));
    assert_eq!(fake.calls().wraps.len(), 1);
}

#[test]
fn streaming_a_replayed_stream_is_not_wrapped_again() {
    let fake = Fake::new();
    let first = replay_cached_result(
        CacheHit::Stored(chat_completion_dict()),
        CachingSupportedCallType::Completion,
        &stream_request(),
        &fake,
    )
    .unwrap();
    let second = replay_cached_result(
        CacheHit::Replayed(first),
        CachingSupportedCallType::Completion,
        &stream_request(),
        &fake,
    )
    .unwrap();
    assert!(matches!(second, Replayed::Stream(_)));
    assert_eq!(fake.calls().wraps.len(), 1);
}

#[test]
fn streaming_a_non_stream_request_never_wraps() {
    let fake = Fake::new();
    let outcome = convert(
        chat_completion_dict(),
        CachingSupportedCallType::Acompletion,
        &non_stream_request(),
        &fake,
    )
    .unwrap();
    assert!(matches!(outcome, Replayed::Response(ResponseKind::Chat, _)));
    assert!(fake.calls().wraps.is_empty());
}
