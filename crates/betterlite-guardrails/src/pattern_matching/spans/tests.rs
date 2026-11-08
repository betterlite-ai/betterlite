#![cfg(test)]

use betterlite_config::sections::content_filter_records::ContentFilterAction;
use fancy_regex::Regex;
use rstest::rstest;

use super::{CompiledPattern, convert_word_numbers, gap_allowed, merge_spans};

#[rstest]
#[case::adjacent("id123", true)]
#[case::nearest_keyword("id more than one word id 123", true)]
#[case::later_keyword("id 1 id 2 123 is id", true)]
#[case::space("id 123", true)]
#[case::one_word("id is 123", true)]
#[case::two_words("id is really 123", false)]
#[case::digits_in_gap("id 2 123", false)]
#[case::keyword_after("123 is id", true)]
#[case::overlap("x123x", true)]
#[case::punctuation("id!? 123", true)]
#[case::no_keyword("123", false)]
#[case::word_numbers("ID one-TWO three", true)]
#[case::word_numbers_wrong_length("id one two", false)]
#[case::word_numbers_blocked_gap("id 4 one two three", false)]
#[case::word_number_boundaries("id someone two three", false)]
fn pattern_matching_behavior(#[case] text: &str, #[case] expected: bool) {
    let pattern = CompiledPattern {
        regex: Regex::new("[0-9]{3}").unwrap(),
        pattern_name: "test".to_owned(),
        action: ContentFilterAction::Mask,
        keyword_regex: Some(Regex::new("(?i)id|x123x").unwrap()),
        allow_word_numbers: true,
    };
    assert_eq!(!pattern.find_spans(text).unwrap().is_empty(), expected);
}

#[path = "tests/gaps/tests.rs"]
mod gaps;

#[path = "tests/merging/tests.rs"]
mod merging;

#[path = "tests/numbers/tests.rs"]
mod numbers;

#[test]
fn word_numbers_require_opt_in() {
    let pattern = CompiledPattern {
        regex: Regex::new("[0-9]{3}").unwrap(),
        pattern_name: "test".to_owned(),
        action: ContentFilterAction::Mask,
        keyword_regex: None,
        allow_word_numbers: false,
    };
    assert!(pattern.find_spans("one two three").unwrap().is_empty());
}
