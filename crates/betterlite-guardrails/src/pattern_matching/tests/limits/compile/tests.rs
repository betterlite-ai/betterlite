#![cfg(test)]

use std::ops::Range;

use super::{ContentFilterAction, ContentFilterPattern, MAX_TEXT_BYTES, PatternError, compile};
use rstest::rstest;

#[rstest]
#[case::below_limit(MAX_TEXT_BYTES.saturating_sub(1), true)]
#[case::at_limit(MAX_TEXT_BYTES, true)]
#[case::above_limit(MAX_TEXT_BYTES.saturating_add(1), false)]
fn pattern_matching_behavior(#[case] length: usize, #[case] accepted: bool) {
    // A long comment tests source size independently of the regex engine's compiled-size limit.
    let source = format!("(?x)x#{}", "a".repeat(length.saturating_sub(6)));
    assert_eq!(source.len(), length);
    let result = compile(&ContentFilterPattern {
        pattern_type: "regex".to_owned(),
        pattern_name: None,
        pattern: Some(source),
        name: None,
        action: ContentFilterAction::Mask,
    });
    if accepted {
        assert_eq!(
            result.unwrap().find_spans("x").unwrap(),
            vec![Range { start: 0, end: 1 }]
        );
    } else {
        assert!(matches!(result.unwrap_err(), PatternError::TooLarge));
    }
}
