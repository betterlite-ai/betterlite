#![cfg(test)]

use betterlite_models::usage_sum::Usage;

use super::support::{frame, object, state_with_pending};
use crate::gemini_live_accounting::usage::{chat_usage, consume_usage_metadata};
use crate::gemini_live_client::state::LiveSessionState;

#[test]
fn a_usage_dictionary_in_the_frame_is_removed_and_clears_the_pending_buffer() {
    let mut state = state_with_pending(r#"{"totalTokenCount": 99}"#);
    let mut message = frame(r#"{"usageMetadata": {"totalTokenCount": 5}}"#);
    let consumed = consume_usage_metadata(&mut state, &mut message);
    assert_eq!(consumed, Some(object(r#"{"totalTokenCount": 5}"#)));
    assert_eq!(message.usage_metadata, None);
    assert_eq!(state.pending_usage_metadata, None);
}

#[test]
fn an_empty_usage_dictionary_in_the_frame_wins_over_a_nonempty_pending_buffer() {
    let mut state = state_with_pending(r#"{"totalTokenCount": 99}"#);
    let mut message = frame(r#"{"usageMetadata": {}}"#);
    let consumed = consume_usage_metadata(&mut state, &mut message);
    assert_eq!(consumed, Some(object("{}")));
    assert_eq!(state.pending_usage_metadata, None);
}

#[test]
fn the_pending_buffer_is_drained_once_when_the_frame_carries_no_usage() {
    let mut state = state_with_pending(r#"{"totalTokenCount": 99}"#);
    let mut message = frame(r#"{"serverContent": {"turnComplete": true}}"#);
    let first = consume_usage_metadata(&mut state, &mut message);
    let second = consume_usage_metadata(&mut state, &mut message);
    assert_eq!(first, Some(object(r#"{"totalTokenCount": 99}"#)));
    assert_eq!(second, None);
}

#[test]
fn a_null_usage_in_the_frame_drains_the_pending_buffer_like_an_absent_one() {
    let mut state = state_with_pending(r#"{"totalTokenCount": 7}"#);
    let mut message = frame(r#"{"usageMetadata": null}"#);
    let consumed = consume_usage_metadata(&mut state, &mut message);
    assert_eq!(consumed, Some(object(r#"{"totalTokenCount": 7}"#)));
    assert_eq!(state.pending_usage_metadata, None);
}

#[test]
fn nothing_reported_and_nothing_pending_consumes_nothing() {
    let mut state = LiveSessionState::default();
    let mut message = frame("{}");
    assert_eq!(consume_usage_metadata(&mut state, &mut message), None);
}

#[test]
fn no_reported_usage_is_zero_usage_without_details() {
    let usage = chat_usage(None, None).expect("zero usage");
    assert_eq!(usage, Usage::default());
}

#[test]
fn reported_usage_goes_through_the_chat_calculation() {
    let reported = object(
        r#"{"promptTokenCount": 10, "candidatesTokenCount": 5, "totalTokenCount": 15,
            "promptTokensDetails": [{"modality": "AUDIO", "tokenCount": 10}]}"#,
    );
    let usage = chat_usage(Some(&reported), None).expect("the usage calculates");
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 15);
    let details = usage.prompt_tokens_details.expect("prompt details");
    assert_eq!(details.audio_tokens, Some(10));
    assert_eq!(details.web_search_requests, None);
}

#[test]
fn search_grounding_in_the_frame_adds_the_web_search_counter() {
    let reported = object(r#"{"promptTokenCount": 1, "totalTokenCount": 1}"#);
    let grounding = object(r#"{"webSearchQueries": ["a", "b", "a"]}"#);
    let usage = chat_usage(Some(&reported), Some(&grounding)).expect("the usage calculates");
    let details = usage.prompt_tokens_details.expect("prompt details");
    assert_eq!(details.web_search_requests, Some(2));
    assert_eq!(details.google_maps_grounding_requests, None);
}

#[test]
fn maps_grounding_in_the_frame_adds_the_maps_counter() {
    let reported = object(r#"{"promptTokenCount": 1, "totalTokenCount": 1}"#);
    let grounding = object(r#"{"groundingChunks": [{"maps": {}}], "webSearchQueries": ["m"]}"#);
    let usage = chat_usage(Some(&reported), Some(&grounding)).expect("the usage calculates");
    let details = usage.prompt_tokens_details.expect("prompt details");
    assert_eq!(details.google_maps_grounding_requests, Some(1));
    assert_eq!(details.web_search_requests, None);
}

#[test]
fn grounding_without_reported_usage_bills_nothing() {
    let grounding = object(r#"{"webSearchQueries": ["a"]}"#);
    let usage = chat_usage(None, Some(&grounding)).expect("zero usage");
    assert_eq!(usage, Usage::default());
}
