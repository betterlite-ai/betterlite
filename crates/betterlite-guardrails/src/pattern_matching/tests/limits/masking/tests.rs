#![cfg(test)]

use super::{MAX_TEXT_BYTES, PatternError, masking};
use rstest::rstest;

#[rstest]
#[case::below_limit(MAX_TEXT_BYTES.saturating_sub(1), true)]
#[case::at_limit(MAX_TEXT_BYTES, true)]
#[case::above_limit(MAX_TEXT_BYTES.saturating_add(1), false)]
fn pattern_matching_behavior(#[case] length: usize, #[case] accepted: bool) {
    let text = "x".repeat(length);
    let result = masking::mask_spans(&text, &[], "");
    if accepted {
        assert_eq!(result.unwrap(), text);
    } else {
        assert!(matches!(result.unwrap_err(), PatternError::TooLarge));
    }
}
