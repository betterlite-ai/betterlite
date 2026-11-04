#![cfg(test)]
use crate::batch::execution::{BatchError, Mode};
use crate::batch::tests::batch;
use std::mem::discriminant;
use tokio::task::JoinSet;

#[rstest::rstest]
#[case::item_limit(0, 1024, 1, BatchError::TooManyItems { actual: 1, maximum: 0 })]
#[case::byte_limit(8, 0, 1, BatchError::TooManyBytes { maximum: 0 })]
#[case::zero_concurrency(8, 1024, 0, BatchError::ZeroConcurrency)]
#[tokio::test]
async fn batch_calls_batch_call_tests_behavior(
    #[case] items: usize,
    #[case] bytes: usize,
    #[case] concurrency: usize,
    #[case] expected: BatchError,
) {
    let mut input = batch(vec![0], Mode::Ordered);
    input.limits.max_items = items;
    input.limits.max_bytes = bytes;
    input.limits.concurrency = concurrency;
    let mut owners = JoinSet::new();
    let result = input.start(
        |_, _| async { Ok::<_, &'static str>(Some(0_u8)) },
        &mut owners,
    );
    let error = result.unwrap_err();
    assert_eq!(discriminant(&error), discriminant(&expected));
    assert_eq!(error.to_string(), expected.to_string());
    assert!(owners.is_empty());
}
