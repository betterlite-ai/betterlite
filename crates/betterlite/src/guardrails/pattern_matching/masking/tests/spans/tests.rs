#![cfg(test)]

use std::ops::Range;

use super::mask_spans;
use crate::guardrails::pattern_matching::PatternError;
use rstest::rstest;

#[rstest]
#[case::whole_text(0, 3, Some("[X]"))]
#[case::ends_at_text_boundary(1, 3, Some("a[X]"))]
#[case::empty_at_start(0, 0, Some("[X]abc"))]
#[case::empty_at_end(3, 3, Some("abc[X]"))]
#[case::reversed(2, 1, None)]
#[case::outside_text(0, 4, None)]
fn pattern_matching_behavior(
    #[case] start: usize,
    #[case] end: usize,
    #[case] expected: Option<&str>,
) {
    let result = mask_spans("abc", &[Range { start, end }], "[X]");
    if let Some(masked) = expected {
        assert_eq!(result.unwrap(), masked);
    } else {
        assert!(matches!(result.unwrap_err(), PatternError::InvalidSpan));
    }
}
