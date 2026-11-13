#![cfg(test)]

use super::{merge_spans, rstest};
use std::ops::Range;

#[rstest]
#[case::overlapping(vec![4..8, 0..6, 2..3], vec![Range { start: 0, end: 8 }])]
#[case::touching(vec![2..4, 0..2], vec![Range { start: 0, end: 4 }])]
#[case::separate(vec![5..7, 0..2], vec![0..2, 5..7])]
#[case::empty(vec![], vec![])]
fn pattern_matching_behavior(
    #[case] spans: Vec<Range<usize>>,
    #[case] expected: Vec<Range<usize>>,
) {
    assert_eq!(merge_spans(spans), expected);
}
