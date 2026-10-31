#![cfg(test)]

use super::interactions::record::{InteractionsUsage, is_interactions_usage_object};
use super::interactions::transform::transform_interactions_usage;
use super::transcription::{
    TranscriptionTokenDetails, TranscriptionTokens, TranscriptionUsage,
    transform_transcription_usage,
};

#[test]
fn transcription_duration_and_tokens_route_through_the_public_api() {
    let duration = TranscriptionUsage::Duration { seconds: 1.0_f64 };
    assert!(transform_transcription_usage(&duration).is_none());

    let tokens = TranscriptionUsage::Tokens(Box::new(TranscriptionTokens {
        input_tokens: 2,
        output_tokens: 1,
        total_tokens: 3,
        input_token_details: TranscriptionTokenDetails {
            audio_tokens: 1,
            text_tokens: 1,
        },
    }));
    let cost = transform_transcription_usage(&tokens).expect("tokens usage transforms");
    assert_eq!(cost.total_tokens, 3);
}

#[test]
fn interactions_recognition_and_transform_route_through_the_public_api() {
    let usage: InteractionsUsage =
        serde_json::from_str(r#"{"total_input_tokens":4,"total_output_tokens":2}"#)
            .expect("valid JSON object");
    assert!(is_interactions_usage_object(&usage));
    let cost = transform_interactions_usage(&usage).expect("valid usage");
    assert_eq!(cost.prompt_tokens, 4);
    assert_eq!(cost.completion_tokens, 2);
    assert_eq!(cost.total_tokens, 6);
}

#[test]
fn chat_shaped_usage_is_not_recognized_as_interactions() {
    let usage: InteractionsUsage =
        serde_json::from_str(r#"{"prompt_tokens":4,"total_input_tokens":4}"#)
            .expect("valid JSON object");
    assert!(!is_interactions_usage_object(&usage));
}
