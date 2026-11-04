#![cfg(test)]

use std::ops::Range;

use crate::guardrails::pattern_matching::{CompiledPattern, python_regex};
use betterlite_config::sections::content_filter_records::ContentFilterAction;
use rstest::rstest;

#[rstest]
#[case::ascii_word_numbers(r"(?a:(1)\1)", "one-one", vec![0..7])]
#[case::ascii_word_mismatch(r"(?a:(1)\1)", "one-two", vec![])]
fn pattern_matching_behavior(
    #[case] source: &str,
    #[case] text: &str,
    #[case] expected: Vec<Range<usize>>,
) {
    let pattern = CompiledPattern {
        regex: python_regex::compile(source).unwrap(),
        pattern_name: "test".to_owned(),
        action: ContentFilterAction::Mask,
        keyword_regex: None,
        allow_word_numbers: true,
    };
    assert_eq!(pattern.find_spans(text).unwrap(), expected);
}
