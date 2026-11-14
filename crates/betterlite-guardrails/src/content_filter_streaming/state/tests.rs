#![cfg(test)]

use betterlite_config::sections::content_filter_records::{
    BlockedWordDetection, ContentFilterDetection,
};

use crate::content_filter_streaming::plan::StreamedScanPlan;
use crate::content_filter_streaming::state::StreamedChoiceState;

#[test]
fn choice_state_starts_with_no_buffer_or_detections() {
    let state = StreamedChoiceState::default();
    assert!(state.buffered_text.is_empty());
    assert_eq!(state.yielded_masked_text_len, 0);
    assert!(state.committed_detections.is_empty());
    assert!(state.latest_detections.is_empty());
    assert_eq!(state.next_trim_len, 0);
}

#[test]
fn choice_state_keeps_the_fields_it_was_built_with() {
    let detection = ContentFilterDetection::BlockedWord(BlockedWordDetection {
        r#type: "blocked_word".to_owned(),
        keyword: "secret".to_owned(),
        action: "MASK".to_owned(),
        description: None,
    });
    let state = StreamedChoiceState {
        buffered_text: "hello secret".to_owned(),
        yielded_masked_text_len: 4,
        committed_detections: vec![detection.clone()],
        latest_detections: vec![detection],
        next_trim_len: 1024,
    };
    assert_eq!(state.buffered_text, "hello secret");
    assert_eq!(state.yielded_masked_text_len, 4);
    assert_eq!(state.committed_detections.len(), 1);
    assert_eq!(state.latest_detections.len(), 1);
    assert_eq!(state.next_trim_len, 1024);
}

#[test]
fn scan_plan_starts_with_no_context_or_phrases() {
    let plan = StreamedScanPlan::default();
    assert_eq!(plan.context_chars, 0);
    assert!(plan.exception_phrases.is_empty());
    assert!(plan.conditional_words.is_empty());
}
