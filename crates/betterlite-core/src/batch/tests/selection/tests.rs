#![cfg(test)]
use crate::batch::selection::{all_responses, first_ordered_model_result};

#[rstest::rstest]
#[case::earlier_error(vec![Err("failed"), Ok(Some(2))], Err("failed"))]
#[case::first_model(vec![Ok(Some(1)), Ok(Some(2))], Ok(Some(1)))]
#[case::skip_none(vec![Ok(None), Ok(Some(2))], Ok(Some(2)))]
#[case::empty(vec![], Ok(None))]
fn batch_calls_batch_call_tests_behavior(
    #[case] input: Vec<Result<Option<u8>, &'static str>>,
    #[case] expected: Result<Option<u8>, &'static str>,
) {
    assert_eq!(first_ordered_model_result(input), expected);
    assert_eq!(
        all_responses(vec![Ok(Some(1_u8)), Err("failed"), Ok(None), Ok(Some(2))]),
        vec![1, 2]
    );
}
