#![cfg(test)]
use crate::batch::{
    execution::{BatchError, Mode},
    tests::batch,
};
use std::time::Duration;
use tokio::task::JoinSet;

#[rstest::rstest]
#[case::empty_models(vec![], Mode::Ordered, Duration::from_secs(1))]
#[case::empty_deployments(vec![], Mode::Race, Duration::from_secs(1))]
#[case::all_deployments_fail(vec![0, 1], Mode::Race, Duration::from_secs(1))]
#[case::deadline_overflow(vec![0], Mode::Ordered, Duration::MAX)]
#[tokio::test]
async fn batch_calls_batch_call_tests_behavior(
    #[case] values: Vec<u8>,
    #[case] mode: Mode,
    #[case] deadline: Duration,
) {
    let mut input = batch(values, mode);
    input.limits.deadline = deadline;
    let mut owners = JoinSet::new();
    let receiver = input.start(|_, _| async { Err::<Option<u8>, _>("failed") }, &mut owners);
    if deadline == Duration::MAX {
        assert!(matches!(receiver, Err(BatchError::DeadlineRange)));
    } else {
        assert!(receiver.unwrap().await.unwrap().unwrap().results.is_empty());
    }
    while let Some(joined) = owners.join_next().await {
        joined.unwrap();
    }
}
