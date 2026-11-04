#![cfg(test)]

use super::{MAX_TEXT_BYTES, blocked_words};
use rstest::rstest;

#[rstest]
#[case::below_limit(MAX_TEXT_BYTES.saturating_sub(1), true)]
#[case::at_limit(MAX_TEXT_BYTES, true)]
#[case::above_limit(MAX_TEXT_BYTES.saturating_add(1), false)]
fn pattern_matching_behavior(#[case] length: usize, #[case] accepted: bool) {
    let prefix = "blocked_words: []\n#";
    let source = format!(
        "{prefix}{}",
        "a".repeat(length.saturating_sub(prefix.len()))
    );
    assert_eq!(source.len(), length);
    let result = blocked_words::decode(source.as_bytes());
    if accepted {
        assert!(result.unwrap().is_empty());
    } else {
        assert!(matches!(
            result.unwrap_err(),
            blocked_words::WordsError::TooLarge
        ));
    }
}
