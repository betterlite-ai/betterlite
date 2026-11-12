use rstest::rstest;

use super::support::{extra, largest_count, usage};
use crate::embedding_merge::error::MergeError;
use crate::embedding_merge::usage::{Usage, combine_usage};

#[test]
fn adds_the_three_counts() {
    let combined =
        combine_usage(&usage([1, 2, 3], None), &usage([10, 20, 30], None)).expect("the counts fit");
    assert_eq!(combined, usage([11, 22, 33], None));
}

#[test]
fn keeps_each_count_in_its_own_field() {
    let combined =
        combine_usage(&usage([1, 0, 0], None), &usage([0, 2, 0], None)).expect("the counts fit");
    assert_eq!(combined, usage([1, 2, 0], None));
}

#[rstest]
#[case::both_absent(None, None, None)]
#[case::only_first(Some(r#"{"cached_tokens": 4}"#), None, Some(r#"{"cached_tokens": 4}"#))]
#[case::only_second(None, Some(r#"{"cached_tokens": 5}"#), Some(r#"{"cached_tokens": 5}"#))]
#[case::both_sum(
    Some(r#"{"cached_tokens": 4, "text_tokens": 1}"#),
    Some(r#"{"cached_tokens": 5}"#),
    Some(r#"{"cached_tokens": 9, "text_tokens": 1}"#)
)]
#[case::both_empty(Some("{}"), Some("{}"), None)]
fn merges_the_prompt_token_details(
    #[case] first: Option<&str>,
    #[case] second: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let combined = combine_usage(&usage([0, 0, 0], first), &usage([0, 0, 0], second))
        .expect("the details merge");
    assert_eq!(combined.prompt_tokens_details, expected.map(extra));
}

#[rstest]
#[case::prompt_tokens([i64::MAX, 0, 0], "prompt_tokens")]
#[case::completion_tokens([0, i64::MAX, 0], "completion_tokens")]
#[case::total_tokens([0, 0, i64::MAX], "total_tokens")]
fn reports_the_field_whose_total_overflows(#[case] counts: [i64; 3], #[case] field: &str) {
    let error = combine_usage(&usage(counts, None), &usage([1, 1, 1], None))
        .expect_err("the total does not fit");
    assert!(matches!(error, MergeError::Overflow { field: reported } if reported == field));
}

#[test]
fn accepts_totals_up_to_the_largest_count() {
    let combined = combine_usage(&usage([i64::MAX - 1, 0, 0], None), &usage([1, 0, 0], None))
        .expect("the total fits");
    assert_eq!(combined.prompt_tokens, i64::MAX);
}

#[test]
fn reports_an_overflow_of_the_details() {
    let mut huge = usage([0, 0, 0], None);
    huge.prompt_tokens_details = Some(largest_count());
    let error = combine_usage(&huge, &usage([0, 0, 0], Some(r#"{"cached_tokens": 1}"#)))
        .expect_err("the details total does not fit");
    assert!(matches!(
        error,
        MergeError::Overflow {
            field: "prompt_tokens_details"
        }
    ));
}

#[test]
fn leaves_both_inputs_unchanged() {
    let first = usage([1, 2, 3], Some(r#"{"cached_tokens": 1}"#));
    let second = usage([4, 5, 6], Some(r#"{"cached_tokens": 2}"#));
    let _combined: Usage = combine_usage(&first, &second).expect("the counts fit");
    assert_eq!(first, usage([1, 2, 3], Some(r#"{"cached_tokens": 1}"#)));
    assert_eq!(second, usage([4, 5, 6], Some(r#"{"cached_tokens": 2}"#)));
}
