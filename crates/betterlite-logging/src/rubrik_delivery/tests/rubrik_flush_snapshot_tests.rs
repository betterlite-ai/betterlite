use std::time::Duration;

use betterlite_logging::rubrik_delivery::queue::{
    append_and_maybe_flush, async_send_batch, flush_queue, queue_length, queue_snapshot,
};
use betterlite_logging::rubrik_delivery::transport::ClientTier;
use betterlite_models::decisions::Json;

use super::support::{
    ScriptedReply, SequenceClock, dropped_since_warning, environment, fill, index_of, init,
    integer, last_flush_time, logger, logger_with_clock, logger_with_environment, object,
};

fn payload_at(index: u64) -> Json {
    object(&[("index", integer(index))])
}

#[tokio::test]
async fn flush_sends_the_snapshot_and_drains_the_sent_prefix() {
    let (logger, transport) = logger(
        &init("https://rubrik.test", "key"),
        vec![ScriptedReply::ok("{}")],
    );
    append_and_maybe_flush(&logger, payload_at(0)).await;
    append_and_maybe_flush(&logger, payload_at(1)).await;
    append_and_maybe_flush(&logger, payload_at(2)).await;
    flush_queue(&logger).await;
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].tier, ClientTier::Logging);
    assert_eq!(
        calls[0].payload,
        Json::List(vec![payload_at(0), payload_at(1), payload_at(2)])
    );
    assert_eq!(queue_length(&logger), 0);
}

#[tokio::test]
async fn failed_send_retains_the_queue() {
    let (logger, transport) = logger(
        &init("https://rubrik.test", "key"),
        vec![ScriptedReply::status(500)],
    );
    append_and_maybe_flush(&logger, payload_at(0)).await;
    append_and_maybe_flush(&logger, payload_at(1)).await;
    flush_queue(&logger).await;
    assert_eq!(transport.call_count(), 1);
    assert_eq!(queue_length(&logger), 2);
    assert_eq!(last_flush_time(&logger), Duration::from_secs(1));
    let queue = queue_snapshot(&logger);
    assert_eq!(queue.first().and_then(index_of), Some(0));
    assert_eq!(queue.last().and_then(index_of), Some(1));
}

#[tokio::test]
async fn payloads_appended_during_send_survive_the_prefix_drain() {
    let (logger, transport) = logger(
        &init("https://rubrik.test", "key"),
        vec![ScriptedReply::ok("{}").pushing(payload_at(2))],
    );
    append_and_maybe_flush(&logger, payload_at(0)).await;
    append_and_maybe_flush(&logger, payload_at(1)).await;
    flush_queue(&logger).await;
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].payload,
        Json::List(vec![payload_at(0), payload_at(1)])
    );
    let queue = queue_snapshot(&logger);
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.first().and_then(index_of), Some(2));
    assert_eq!(queue_length(&logger), 1);
}

#[tokio::test]
async fn events_appended_during_an_overflowing_send_all_survive() {
    let values = environment(&[("RUBRIK_BATCH_SIZE", "20000")]);
    let (logger, transport) = logger_with_environment(
        &init("https://rubrik.test", "key"),
        &values,
        vec![
            ScriptedReply::ok("{}")
                .pushing(payload_at(10_000))
                .pushing(payload_at(10_001))
                .pushing(payload_at(10_002)),
        ],
    );
    fill(&logger, 0, 10_000);
    flush_queue(&logger).await;
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    match &calls[0].payload {
        Json::List(sent) => {
            assert_eq!(sent.len(), 10_000);
            assert_eq!(sent.first().and_then(index_of), Some(0));
            assert_eq!(sent.last().and_then(index_of), Some(9_999));
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            panic!("the batch payload is a JSON array")
        }
    }
    assert_eq!(dropped_since_warning(&logger), 3);
    let queue = queue_snapshot(&logger);
    let indices: Vec<Option<u64>> = queue.iter().map(index_of).collect();
    assert_eq!(indices, vec![Some(10_000), Some(10_001), Some(10_002)]);
}

#[tokio::test]
async fn send_batch_delivers_without_draining() {
    let (logger, transport) = logger(
        &init("https://rubrik.test", "key"),
        vec![ScriptedReply::ok("{}")],
    );
    append_and_maybe_flush(&logger, payload_at(0)).await;
    append_and_maybe_flush(&logger, payload_at(1)).await;
    async_send_batch(&logger)
        .await
        .expect("the batch send succeeds");
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].payload,
        Json::List(vec![payload_at(0), payload_at(1)])
    );
    assert_eq!(queue_length(&logger), 2);
}

#[tokio::test]
async fn successful_flush_records_the_clock_time() {
    let (logger, transport) = logger_with_clock(
        &init("https://rubrik.test", "key"),
        SequenceClock::new(&[Duration::from_secs(10), Duration::from_secs(42)]),
        vec![ScriptedReply::ok("{}")],
    );
    assert_eq!(last_flush_time(&logger), Duration::from_secs(10));
    append_and_maybe_flush(&logger, payload_at(0)).await;
    flush_queue(&logger).await;
    assert_eq!(last_flush_time(&logger), Duration::from_secs(42));
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn empty_flush_sends_nothing() {
    let (logger, transport) = logger(&init("https://rubrik.test", "key"), Vec::new());
    flush_queue(&logger).await;
    assert_eq!(transport.call_count(), 0);
    assert_eq!(queue_length(&logger), 0);
}

#[tokio::test]
async fn flush_retries_the_retained_events_after_a_failure() {
    let (logger, transport) = logger(
        &init("https://rubrik.test", "key"),
        vec![ScriptedReply::status(500), ScriptedReply::ok("{}")],
    );
    append_and_maybe_flush(&logger, payload_at(0)).await;
    append_and_maybe_flush(&logger, payload_at(1)).await;
    flush_queue(&logger).await;
    assert_eq!(queue_length(&logger), 2);
    flush_queue(&logger).await;
    let calls = transport.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[1].payload,
        Json::List(vec![payload_at(0), payload_at(1)])
    );
    assert_eq!(queue_length(&logger), 0);
}
