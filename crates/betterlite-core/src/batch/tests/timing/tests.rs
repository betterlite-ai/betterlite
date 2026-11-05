#![cfg(test)]
use crate::batch::{execution::Mode, selection::first_ordered_model_result, tests::batch};
use std::{sync::Arc, time::Duration};
use tokio::{sync::Notify, task::JoinSet, time::timeout};

#[rstest::rstest]
#[case::models_wait_for_first_future(0)]
#[case::models_join_later_work(1)]
#[tokio::test(start_paused = true)]
async fn batch_calls_batch_call_tests_behavior(#[case] blocked: u8) {
    let gate = Arc::new(Notify::new());
    let release = Arc::clone(&gate);
    let mut owners = JoinSet::new();
    let mut receiver = batch(vec![0, 1], Mode::Ordered)
        .start(
            move |value, _| {
                let wait = Arc::clone(&gate);
                async move {
                    if value == blocked {
                        wait.notified().await;
                    }
                    Ok::<_, &'static str>(Some(value))
                }
            },
            &mut owners,
        )
        .unwrap();
    timeout(Duration::from_secs(1), &mut receiver)
        .await
        .unwrap_err();
    release.notify_one();
    let report = receiver.await.unwrap().unwrap();
    assert_eq!(first_ordered_model_result(report.results), Ok(Some(0)));
    while let Some(joined) = owners.join_next().await {
        joined.unwrap();
    }
}
