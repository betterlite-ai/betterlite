#![cfg(test)]

use std::ops::Range;

use super::{ContentFilterAction, ContentFilterPattern, compile, rstest};
use crate::guardrails::pattern_matching::masking::mask_spans;

#[rstest]
#[case::lazy_dot(".*?", "ab", vec![0..2], "[X]")]
#[case::lazy_unicode(".*?", "\u{e9}\u{4e2d}", vec![0..5], "[X]")]
#[case::empty_only("(?=a)", "ab", vec![0..0], "[X]ab")]
#[case::empty_everywhere("x*", "ab", vec![0..0, 1..1, 2..2], "[X]a[X]b[X]")]
#[case::later_consuming("|b", "ab", vec![0..0, 1..2], "[X]a[X]")]
#[case::empty_text(".*?", "", vec![0..0], "[X]")]
#[case::lazy_choice("a*?|b", "ab", vec![0..2], "[X]")]
#[case::lookbehind("(?<=a).*?", "abc", vec![1..2], "a[X]c")]
fn pattern_matching_behavior(
    #[case] source: &str,
    #[case] text: &str,
    #[case] expected: Vec<Range<usize>>,
    #[case] masked: &str,
) {
    let pattern = compile(&ContentFilterPattern {
        pattern_type: "regex".to_owned(),
        pattern_name: None,
        pattern: Some(source.to_owned()),
        name: None,
        action: ContentFilterAction::Mask,
    })
    .unwrap();
    let spans = pattern.find_spans(text).unwrap();
    assert_eq!(spans, expected);
    assert_eq!(mask_spans(text, &spans, "[X]").unwrap(), masked);
}

#[rstest]
#[case::unsupported_variable_lookbehind("(?<=a*)b|(?=c)", "ac")]
fn retry_propagates_unsupported_compile_errors(#[case] source: &str, #[case] text: &str) {
    // The retry rebuilds the whole pattern with find_not_empty, where this lookbehind becomes unsupported.
    let pattern = compile(&ContentFilterPattern {
        pattern_type: "regex".to_owned(),
        pattern_name: None,
        pattern: Some(source.to_owned()),
        name: None,
        action: ContentFilterAction::Mask,
    })
    .unwrap();
    assert_eq!(
        pattern.find_spans(text).unwrap_err().to_string(),
        "invalid regex or matching failed: Error compiling regex: \
         Regex uses currently unimplemented feature: \
         Variable length lookbehinds with fancy features"
    );
}
