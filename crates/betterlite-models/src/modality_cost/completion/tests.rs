#![cfg(test)]

use rstest::rstest;

use super::parse_completion_details;
use crate::cost_usage::CompletionDetails;

fn empty_completion_details() -> CompletionDetails {
    CompletionDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        reasoning_tokens: None,
    }
}

#[test]
fn reasoning_nested_in_text_is_removed_up_to_the_excess() {
    let details = CompletionDetails {
        text_tokens: Some(10),
        reasoning_tokens: Some(4),
        ..empty_completion_details()
    };
    // Reported total (10+4) exceeds completion_tokens (12) by 2, so only 2 of the 4 reasoning tokens are nested.
    let result = parse_completion_details(12, &details).expect("no overflow");
    assert_eq!(result.text_tokens, 8);
    assert_eq!(result.reasoning_tokens, 4);
}

#[test]
fn no_excess_removes_no_reasoning() {
    let details = CompletionDetails {
        text_tokens: Some(10),
        reasoning_tokens: Some(4),
        ..empty_completion_details()
    };
    let result = parse_completion_details(20, &details).expect("no overflow");
    assert_eq!(result.text_tokens, 10);
}

#[test]
fn other_modalities_count_toward_the_excess() {
    let details = CompletionDetails {
        text_tokens: Some(5),
        reasoning_tokens: Some(5),
        audio_tokens: Some(6),
        ..empty_completion_details()
    };
    // Reported total (5+5+6=16) exceeds completion_tokens (10) by 6, capped by min(reasoning=5, text=5).
    let result = parse_completion_details(10, &details).expect("no overflow");
    assert_eq!(result.text_tokens, 0);
}

#[test]
fn missing_text_tokens_default_to_zero() {
    let result = parse_completion_details(5, &empty_completion_details()).expect("ok");
    assert_eq!(result.text_tokens, 0);
}

#[rstest]
#[case::absent_defaults_to_zero(None, 0)]
#[case::zero_stays_zero(Some(0), 0)]
#[case::negative_collapses_to_zero(Some(-2), 0)]
#[case::positive_is_kept(Some(3), 3)]
fn video_tokens_require_a_strictly_positive_count(
    #[case] reported: Option<i64>,
    #[case] expected: i64,
) {
    let details = CompletionDetails {
        video_tokens: reported,
        ..empty_completion_details()
    };
    let result = parse_completion_details(0, &details).expect("ok");
    assert_eq!(result.video_tokens, expected);
}

#[test]
fn audio_and_image_counters_pass_through_unchanged() {
    let details = CompletionDetails {
        audio_tokens: Some(7),
        image_tokens: Some(2),
        ..empty_completion_details()
    };
    let result = parse_completion_details(9, &details).expect("ok");
    assert_eq!(result.audio_tokens, 7);
    assert_eq!(result.image_tokens, 2);
}
