#![cfg(test)]

use super::{
    TranscriptionTokenDetails, TranscriptionTokens, TranscriptionUsage, TranscriptionUsageError,
    transform_transcription_usage,
};

#[test]
fn duration_usage_transforms_to_none() {
    let usage = TranscriptionUsage::Duration { seconds: 12.5_f64 };
    assert!(transform_transcription_usage(&usage).is_none());
}

#[test]
fn token_usage_copies_counts_without_recomputing() {
    let usage = TranscriptionUsage::Tokens(Box::new(TranscriptionTokens {
        input_tokens: 10,
        output_tokens: 3,
        total_tokens: 999,
        input_token_details: TranscriptionTokenDetails {
            audio_tokens: 7,
            text_tokens: 3,
        },
    }));
    let cost = transform_transcription_usage(&usage).expect("tokens usage transforms");
    assert_eq!(cost.prompt_tokens, 10);
    assert_eq!(cost.completion_tokens, 3);
    assert_eq!(cost.total_tokens, 999);
    assert!(cost.completion_details.is_none());
    assert!(cost.cache_read_tokens.is_none());
    let details = cost
        .prompt_details
        .expect("tokens usage has prompt details");
    assert_eq!(details.audio_tokens, Some(7));
    assert_eq!(details.text_tokens, Some(3));
}

#[test]
fn duration_usage_deserializes_from_json() {
    let usage: TranscriptionUsage =
        serde_json::from_str(r#"{"type":"duration","seconds":1.5}"#).expect("valid JSON");
    assert!(transform_transcription_usage(&usage).is_none());
}

#[test]
fn missing_seconds_is_a_typed_error() {
    let error = serde_json::from_str::<TranscriptionUsage>(r#"{"type":"duration"}"#)
        .unwrap_err()
        .to_string();
    assert!(error.contains("missing field `seconds`"));
}

#[test]
fn missing_token_fields_are_typed_errors() {
    use super::RawTranscriptionUsage;

    let missing_input = RawTranscriptionUsage {
        kind: super::TranscriptionUsageKind::Tokens,
        seconds: None,
        input_tokens: None,
        output_tokens: Some(1),
        total_tokens: Some(1),
        input_token_details: Some(TranscriptionTokenDetails {
            audio_tokens: 0,
            text_tokens: 0,
        }),
    };
    assert_eq!(
        TranscriptionUsage::try_from(missing_input).unwrap_err(),
        TranscriptionUsageError::MissingInputTokens
    );
}
