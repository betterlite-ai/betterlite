#![cfg(test)]

use std::ops::Range;

use super::{ContentFilterAction, ContentFilterPattern, MAX_TEXT_BYTES, PatternError, compile};
use rstest::rstest;

#[rstest]
#[case::below_limit(MAX_TEXT_BYTES.saturating_sub(1), true)]
#[case::at_limit(MAX_TEXT_BYTES, true)]
#[case::above_limit(MAX_TEXT_BYTES.saturating_add(1), false)]
fn pattern_matching_behavior(#[case] length: usize, #[case] accepted: bool) {
    let pattern = compile(&ContentFilterPattern {
        pattern_type: "regex".to_owned(),
        pattern_name: None,
        pattern: Some("x+".to_owned()),
        name: None,
        action: ContentFilterAction::Mask,
    })
    .unwrap();
    let result = pattern.find_spans(&"x".repeat(length));
    if accepted {
        assert_eq!(
            result.unwrap(),
            vec![Range {
                start: 0,
                end: length
            }]
        );
    } else {
        assert!(matches!(result.unwrap_err(), PatternError::TooLarge));
    }
}
