#![cfg(test)]
use crate::batch::execution::Mode;
use crate::batch::tests::{Finished, after_cleanup, batch};
use std::{
    future::pending,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
use tokio::{
    sync::{Notify, oneshot},
    task::JoinSet,
};

#[rstest::rstest]
#[case::race_none_is_a_success(None)]
#[case::race_joins_losers(Some(7))]
#[tokio::test]
async fn batch_calls_batch_call_tests_behavior(#[case] value: Option<u8>) {
    let mut owners = JoinSet::new();
    let dropped = Arc::new(AtomicBool::new(false));
    let child_dropped = Arc::clone(&dropped);
    let (finished, mut observed) = oneshot::channel();
    let finish = Mutex::new(Some(finished));
    let gate = Arc::new(Notify::new());
    let receiver = batch(vec![0, 1], Mode::Race)
        .start(
            move |index, _| {
                let ready = Arc::clone(&gate);
                let finished_flag = Arc::clone(&child_dropped);
                let signal = if index == 0 {
                    finish.lock().unwrap().take()
                } else {
                    None
                };
                async move {
                    if index == 0 {
                        let _guard = Finished(signal, finished_flag);
                        ready.notify_one();
                        pending::<()>().await;
                    } else {
                        ready.notified().await;
                    }
                    Ok::<_, &'static str>(value)
                }
            },
            &mut owners,
        )
        .unwrap();
    assert_eq!(
        after_cleanup(receiver, dropped)
            .await
            .unwrap()
            .unwrap()
            .results,
        vec![Ok(value)]
    );
    observed.try_recv().unwrap();
    while let Some(joined) = owners.join_next().await {
        joined.unwrap();
    }
}
