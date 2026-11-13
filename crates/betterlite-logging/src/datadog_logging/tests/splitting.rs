use std::sync::atomic::AtomicUsize;

use crate::batch::delivery::BatchDelivery as _;
use crate::callback_logger::CallbackLogger as _;
use crate::datadog::payloads::constants::DATADOG_413_ERROR;
use crate::datadog::payloads::types::DatadogPayload;
use crate::datadog_logging::batch;
use crate::datadog_logging::logger::DatadogLogger;
use crate::datadog_logging::send::{DatadogBatch, exceeds_intake_limits};
use crate::datadog_logging::settings::{DD_MAX_BATCH_SIZE, DD_MAX_PAYLOAD_SIZE_BYTES, DatadogInit};
use crate::datadog_logging::tests::support;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

fn direct() -> DatadogInit {
    support::direct_init("key-1", "us5.datadoghq.com")
}

async fn deliver(logger: &DatadogLogger, batch_of: &[DatadogPayload]) -> Vec<DatadogPayload> {
    let delivery = DatadogBatch { logger };
    let settled = AtomicUsize::new(0);
    delivery
        .deliver(batch_of, &settled)
        .await
        .expect("the delivery completes")
}

fn sizes(transport: &support::ScriptedTransport) -> Vec<usize> {
    transport
        .calls()
        .iter()
        .map(|call| call.entries().len())
        .collect()
}

async fn queue(logger: &DatadogLogger, ids: &[&str]) {
    for id in ids {
        logger
            .async_log_success_event(support::event(support::standard_payload(id, false)))
            .await
            .expect("the async event is accepted");
    }
}

#[tokio::test]
async fn datadog_413_splitting_behavior() {
    let (logger, transport) = support::build(
        &direct(),
        &support::Environment::new(),
        vec![
            support::Reply::status(413),
            support::Reply::accepted(),
            support::Reply::accepted(),
        ],
    );
    queue(&logger, &["a", "b", "c", "d"]).await;
    batch::flush_queue(&logger).await;
    assert_eq!(sizes(&transport), vec![4, 2, 2]);
    assert_eq!(logger.queue.pending().len(), 0);

    let (_guard, logs) = support::capture_logs();
    let (second_logger, second_transport) = support::build(
        &direct(),
        &support::Environment::new(),
        vec![
            support::Reply::status(413),
            support::Reply::status(413),
            support::Reply::accepted(),
        ],
    );
    queue(&second_logger, &["a", "b"]).await;
    batch::flush_queue(&second_logger).await;
    assert_eq!(sizes(&second_transport), vec![2, 1, 1]);
    assert_eq!(second_logger.queue.pending().len(), 0);
    assert_eq!(logs.count_containing(DATADOG_413_ERROR), 1);
}

#[tokio::test]
async fn datadog_413_splitting_is_proactive_for_a_large_event_count() {
    let (logger, transport) = support::build(
        &direct(),
        &support::Environment::new(),
        vec![support::Reply::accepted(), support::Reply::accepted()],
    );
    let many = vec![support::simple_payload(); DD_MAX_BATCH_SIZE + 1];
    let undelivered = deliver(&logger, &many).await;
    assert!(undelivered.is_empty());
    assert_eq!(sizes(&transport), vec![500, 501]);
}

#[tokio::test]
async fn datadog_413_splitting_is_proactive_for_a_large_payload() {
    let (logger, transport) = support::build(
        &direct(),
        &support::Environment::new(),
        vec![support::Reply::accepted(), support::Reply::accepted()],
    );
    let heavy = vec![
        support::oversized_payload(2_100_000),
        support::oversized_payload(2_100_000),
    ];
    let heavy_undelivered = deliver(&logger, &heavy).await;
    assert!(heavy_undelivered.is_empty());
    assert_eq!(sizes(&transport), vec![1, 1]);

    let (lone_logger, lone_transport) = support::build(
        &direct(),
        &support::Environment::new(),
        vec![support::Reply::accepted()],
    );
    let lone = vec![support::oversized_payload(4_100_000)];
    let lone_undelivered = deliver(&lone_logger, &lone).await;
    assert!(lone_undelivered.is_empty());
    assert_eq!(sizes(&lone_transport), vec![1]);
}

#[test]
fn datadog_413_splitting_limits_are_exclusive() {
    let at_count_limit = vec![support::simple_payload(); DD_MAX_BATCH_SIZE];
    assert!(!exceeds_intake_limits(&at_count_limit).expect("the batch is measurable"));
    let past_count_limit = vec![support::simple_payload(); DD_MAX_BATCH_SIZE + 1];
    assert!(exceeds_intake_limits(&past_count_limit).expect("the batch is measurable"));

    let empty = support::oversized_payload(0);
    let overhead = safe_dumps(&[empty], DEFAULT_MAX_RECURSE_DEPTH, None)
        .expect("the batch is serializable")
        .len();
    let exact = vec![support::oversized_payload(
        DD_MAX_PAYLOAD_SIZE_BYTES - overhead,
    )];
    let encoded =
        safe_dumps(&exact, DEFAULT_MAX_RECURSE_DEPTH, None).expect("the batch is serializable");
    assert_eq!(encoded.len(), DD_MAX_PAYLOAD_SIZE_BYTES);
    assert!(!exceeds_intake_limits(&exact).expect("the batch is measurable"));
    let above = vec![support::oversized_payload(
        DD_MAX_PAYLOAD_SIZE_BYTES + 1 - overhead,
    )];
    assert!(exceeds_intake_limits(&above).expect("the batch is measurable"));
}
