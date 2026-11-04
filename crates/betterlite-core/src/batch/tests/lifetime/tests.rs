#![cfg(test)]
use crate::batch::execution::{BatchError, Mode};
use crate::batch::tests::{Finished, batch};
use std::{future::pending, sync::Mutex};
use tokio::{sync::oneshot, task::JoinSet};

#[rstest::rstest]
#[case::dropped_caller_joins_children(false)]
#[case::deadline_joins_children(true)]
#[tokio::test(start_paused = true)]
async fn batch_calls_batch_call_tests_behavior(#[case] deadline: bool) {
    let mut owners = JoinSet::new();
    let (started, ready) = oneshot::channel();
    let (finished, observed) = oneshot::channel();
    let input = batch(vec![0], Mode::Ordered);
    let senders = Mutex::new(Some((started, finished)));
    let receiver = input
        .start(
            move |_, _| {
                let (start, finish) = senders.lock().unwrap().take().unwrap();
                async move {
                    let _guard = Finished(Some(finish));
                    start.send(()).unwrap();
                    pending::<Result<Option<u8>, &'static str>>().await
                }
            },
            &mut owners,
        )
        .unwrap();
    ready.await.unwrap();
    if deadline {
        assert!(matches!(receiver.await.unwrap(), Err(BatchError::Deadline)));
    } else {
        drop(receiver);
    }
    while let Some(joined) = owners.join_next().await {
        joined.unwrap();
    }
    observed.await.unwrap();
}
