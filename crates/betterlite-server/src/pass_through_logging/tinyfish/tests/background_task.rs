#![cfg(test)]

use super::billing::AsyncRun;
use super::support::{
    ScriptedReply, dispatch_handle, fixture, json, kwargs, shared_logging_object, start_time,
};
use crate::pass_through_logging::background_tasks::BackgroundTasks;

const PENDING_BODY: &str = r#"{"run_id": "run-1", "status": "PENDING"}"#;
const COMPLETED_BODY: &str = r#"{"run_id": "run-1", "status": "COMPLETED", "num_of_steps": 3}"#;

#[tokio::test]
async fn the_async_poller_is_detached_from_the_caller() {
    let fixture = fixture(vec![ScriptedReply::Hanging]);
    let body = json(PENDING_BODY);
    let handle = fixture.billing.start_async_run_billing(AsyncRun {
        logging_obj: shared_logging_object(),
        response_body: Some(&body),
        result: String::new(),
        kwargs: kwargs(&[]),
        log: dispatch_handle(&fixture.dispatch),
        start_time: start_time(),
    });

    assert!(!handle.is_finished());
    assert!(!fixture.tasks.is_empty());
    handle.abort();
    assert!(
        handle
            .await
            .expect_err("the poller was cancelled")
            .is_cancelled()
    );
    assert!(fixture.tasks.is_empty());
}

#[tokio::test]
async fn a_finished_poller_leaves_the_tracker_empty() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let body = json(PENDING_BODY);
    fixture
        .billing
        .start_async_run_billing(AsyncRun {
            logging_obj: shared_logging_object(),
            response_body: Some(&body),
            result: String::new(),
            kwargs: kwargs(&[]),
            log: dispatch_handle(&fixture.dispatch),
            start_time: start_time(),
        })
        .await
        .expect("the poller finishes");

    assert!(fixture.tasks.is_empty());
    assert_eq!(fixture.dispatch.calls().len(), 1);
}

// The server drains its own tracker at shutdown, so a live poller is waited for rather than dropped.
#[tokio::test]
async fn shutdown_waits_for_a_running_poller() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let body = json(PENDING_BODY);
    let handle = fixture.billing.start_async_run_billing(AsyncRun {
        logging_obj: shared_logging_object(),
        response_body: Some(&body),
        result: String::new(),
        kwargs: kwargs(&[]),
        log: dispatch_handle(&fixture.dispatch),
        start_time: start_time(),
    });

    BackgroundTasks {
        tasks: fixture.tasks.clone(),
    }
    .shutdown()
    .await;

    assert!(fixture.tasks.is_empty());
    assert_eq!(fixture.dispatch.calls().len(), 1);
    handle.await.expect("the poller finishes");
}
