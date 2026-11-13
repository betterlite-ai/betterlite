#![cfg(test)]
use crate::batch::{
    execution::{BatchError, Mode},
    tests::batch,
};
use tokio::task::JoinSet;

#[rstest::rstest]
#[case::exact_byte_and_item_limits(0)]
#[case::total_bytes_exceed_limit(1)]
#[tokio::test]
async fn batch_calls_batch_call_tests_behavior(#[case] missing: usize) {
    let mut input = batch(vec![0, 1], Mode::Ordered);
    input.limits.max_items = 2;
    let bytes: usize = input
        .items
        .iter()
        .map(|item| serde_json::to_vec(item).unwrap().len())
        .sum();
    input.limits.max_bytes = bytes.checked_sub(missing).unwrap();
    let mut owners = JoinSet::new();
    let result = input.start(
        |value, _| async move { Ok::<_, &'static str>(Some(value)) },
        &mut owners,
    );
    if missing == 0 {
        assert_eq!(
            result.unwrap().await.unwrap().unwrap().results,
            vec![Ok(Some(0)), Ok(Some(1))]
        );
    } else {
        assert!(matches!(result, Err(BatchError::TooManyBytes { .. })));
    }
    while let Some(joined) = owners.join_next().await {
        joined.unwrap();
    }
}
