use std::time::Duration;

use tokio::time::timeout;

use crate::callback_logger::CallbackLogger as _;
use crate::datadog_logging::batch::{self, NO_BATCH};
use crate::datadog_logging::logger::DatadogLogger;
use crate::datadog_logging::settings::DatadogInit;
use crate::datadog_logging::tests::support;

fn direct() -> DatadogInit {
    support::direct_init("key-1", "us5.datadoghq.com")
}

async fn queue(logger: &DatadogLogger, ids: &[&str]) {
    for id in ids {
        logger
            .async_log_success_event(support::event(support::standard_payload(id, false)))
            .await
            .expect("the async event is accepted");
    }
}

async fn send_once(reply: support::Reply) -> (usize, Vec<String>) {
    let (logger, _transport) = support::build(&direct(), &support::Environment::new(), vec![reply]);
    queue(&logger, &["a"]).await;
    let report = batch::async_send_batch(&logger).await;
    (report.requeued, support::message_ids(&logger))
}

#[tokio::test]
async fn datadog_error_requeue_behavior() {
    let (network_requeued, network_queue) = send_once(support::Reply::network()).await;
    assert_eq!(network_requeued, 1);
    assert_eq!(network_queue, vec!["a"]);

    let (server_requeued, server_queue) = send_once(support::Reply::status(500)).await;
    assert_eq!(server_requeued, 1);
    assert_eq!(server_queue, vec!["a"]);

    let (client_requeued, client_queue) = send_once(support::Reply::status(400)).await;
    assert_eq!(client_requeued, 1);
    assert_eq!(client_queue, vec!["a"]);

    let (extra_requeued, extra_queue) = send_once(support::Reply::status(200)).await;
    assert_eq!(extra_requeued, 1);
    assert_eq!(extra_queue, vec!["a"]);
}

#[tokio::test]
async fn datadog_error_requeue_accepts_a_202() {
    let (logger, transport) = support::build(
        &direct(),
        &support::Environment::new(),
        vec![support::Reply::accepted()],
    );
    queue(&logger, &["a"]).await;
    let report = batch::async_send_batch(&logger).await;
    assert_eq!(report.delivered, 1);
    assert_eq!(report.requeued, 0);
    assert!(support::message_ids(&logger).is_empty());
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn datadog_error_requeue_leaves_only_the_undelivered_half() {
    let (logger, transport) = support::build(
        &direct(),
        &support::Environment::new(),
        vec![
            support::Reply::status(413),
            support::Reply::accepted(),
            support::Reply::status(500),
        ],
    );
    queue(&logger, &["a", "b", "c", "d"]).await;
    batch::flush_queue(&logger).await;
    let sizes: Vec<usize> = transport
        .calls()
        .iter()
        .map(|call| call.entries().len())
        .collect();
    assert_eq!(sizes, vec![4, 2, 2]);
    assert_eq!(support::message_ids(&logger), vec!["c", "d"]);
}

#[tokio::test]
async fn datadog_error_requeue_keeps_events_when_the_send_is_cancelled() {
    let (logger, _transport) = support::build(
        &direct(),
        &support::Environment::new(),
        vec![support::Reply::pending()],
    );
    queue(&logger, &["a", "b"]).await;
    let sending = batch::async_send_batch(&logger);
    let timed_out = timeout(Duration::from_millis(20), sending).await;
    assert!(timed_out.is_err(), "the scripted send never answers");
    assert_eq!(support::message_ids(&logger), vec!["a", "b"]);
}

#[tokio::test]
async fn an_empty_send_reports_no_batch() {
    let (logger, transport) = support::build(&direct(), &support::Environment::new(), Vec::new());
    let report = batch::async_send_batch(&logger).await;
    assert_eq!(report, NO_BATCH);
    assert_eq!(transport.call_count(), 0);
}
