#![cfg(test)]

use std::future::pending;
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::time::timeout;

use super::BILLING_CANCELLED_WARNING;
use super::support::capture_logs;
use crate::pass_through_logging::background_tasks::BackgroundTasks;

#[tokio::test]
async fn holds_a_billing_task_until_it_finishes() {
    let tasks = BackgroundTasks::default();
    let (sender, receiver) = oneshot::channel::<()>();
    let handle = tasks.spawn(
        async move {
            drop(receiver.await);
        },
        BILLING_CANCELLED_WARNING.to_owned(),
    );

    assert!(!handle.is_finished());
    sender.send(()).expect("the poller is waiting");
    handle.await.expect("the poller finishes");
}

#[tokio::test]
async fn a_cancelled_poller_warns_that_the_run_may_go_unbilled() {
    let (logs, _guard) = capture_logs();
    let tasks = BackgroundTasks::default();
    let (started, started_receiver) = oneshot::channel::<()>();
    let handle = tasks.spawn(
        async move {
            started.send(()).expect("the poller started");
            pending::<()>().await;
        },
        BILLING_CANCELLED_WARNING.to_owned(),
    );

    started_receiver.await.expect("the poller started");
    handle.abort();
    let error = handle.await.expect_err("the poller was cancelled");

    assert!(error.is_cancelled());
    assert!(
        logs.contents()
            .contains("billing poller cancelled mid-poll")
    );
    assert!(logs.contents().contains("the run may go unbilled"));
}

#[tokio::test]
async fn a_finished_poller_warns_about_nothing() {
    let (logs, _guard) = capture_logs();
    let tasks = BackgroundTasks::default();
    let (sender, receiver) = oneshot::channel::<()>();
    let handle = tasks.spawn(
        async move {
            drop(receiver.await);
        },
        BILLING_CANCELLED_WARNING.to_owned(),
    );

    drop(sender);
    handle.await.expect("the poller finishes");

    assert!(!logs.contents().contains("cancelled mid-poll"));
}

#[tokio::test]
async fn shutdown_waits_for_the_pending_pollers() {
    let tasks = BackgroundTasks::default();
    let (sender, receiver) = oneshot::channel::<()>();
    let (done, done_receiver) = oneshot::channel::<()>();
    let handle = tasks.spawn(
        async move {
            drop(receiver.await);
            done.send(()).expect("the poller recorded its completion");
        },
        BILLING_CANCELLED_WARNING.to_owned(),
    );

    let mut shutdown = std::pin::pin!(tasks.shutdown());
    let early = timeout(Duration::from_millis(10), &mut shutdown).await;
    assert!(early.is_err());
    drop(sender);
    shutdown.await;
    done_receiver
        .await
        .expect("the poller recorded its completion");
    handle.await.expect("the poller finished");
}
