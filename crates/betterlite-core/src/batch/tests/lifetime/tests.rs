#![cfg(test)]
use crate::batch::execution::{BatchError, Mode};
use crate::batch::tests::{Finished, after_cleanup, batch};
use std::{
    future::pending,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
use tokio::{sync::oneshot, task::JoinSet};

#[rstest::rstest]
#[case::dropped_caller_joins_children(false)]
#[case::deadline_joins_children(true)]
#[tokio::test(start_paused = true)]
async fn batch_calls_batch_call_tests_behavior(#[case] deadline: bool) {
    let mut owners = JoinSet::new();
    let dropped = Arc::new(AtomicBool::new(false));
    let child_dropped = Arc::clone(&dropped);
    let (started, ready) = oneshot::channel();
    let (finished, mut observed) = oneshot::channel();
    let input = batch(vec![0], Mode::Ordered);
    let senders = Mutex::new(Some((started, finished)));
    let receiver = input
        .start(
            move |_, _| {
                let (start, finish) = senders.lock().unwrap().take().unwrap();
                let finished_flag = Arc::clone(&child_dropped);
                async move {
                    let _guard = Finished(Some(finish), finished_flag);
                    start.send(()).unwrap();
                    pending::<Result<Option<u8>, &'static str>>().await
                }
            },
            &mut owners,
        )
        .unwrap();
    ready.await.unwrap();
    if deadline {
        assert!(matches!(
            after_cleanup(receiver, Arc::clone(&dropped)).await.unwrap(),
            Err(BatchError::Deadline)
        ));
    } else {
        drop(receiver);
    }
    while let Some(joined) = after_cleanup(owners.join_next(), Arc::clone(&dropped)).await {
        joined.unwrap();
    }
    observed.try_recv().unwrap();
}
