#![cfg(test)]

use super::{MAX_TEXT_BYTES, masking};
use rstest::rstest;

#[rstest]
#[case::format_below_limit(MAX_TEXT_BYTES.saturating_sub(1), 0, true)]
#[case::format_at_limit(MAX_TEXT_BYTES, 0, true)]
#[case::format_above_limit(MAX_TEXT_BYTES.saturating_add(1), 0, false)]
#[case::name_below_limit(0, MAX_TEXT_BYTES.saturating_sub(1), true)]
#[case::name_at_limit(0, MAX_TEXT_BYTES, true)]
#[case::name_above_limit(0, MAX_TEXT_BYTES.saturating_add(1), false)]
fn pattern_matching_behavior(
    #[case] format_length: usize,
    #[case] name_length: usize,
    #[case] accepted: bool,
) {
    let format = "x".repeat(format_length);
    let result = masking::redaction_tag(&format, &"x".repeat(name_length));
    if accepted {
        assert_eq!(result.unwrap(), format);
    } else {
        assert!(matches!(result.unwrap_err(), masking::MaskError::TooLarge));
    }
}
