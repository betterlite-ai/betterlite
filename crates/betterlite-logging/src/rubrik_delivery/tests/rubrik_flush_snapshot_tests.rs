use std::time::Duration;

use betterlite_logging::rubrik_delivery::queue::{
    append_and_maybe_flush, async_send_batch, flush_queue, queue_snapshot,
};
use betterlite_logging::rubrik_delivery::transport::ClientTier;
use betterlite_models::decisions::Json;

use super::support::{
    ScriptedReply, SequenceClock, index_of, init, integer, logger, logger_with_clock, object,
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
    assert_eq!(logger.state().length, 0);
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
    let state = logger.state();
    assert_eq!(state.length, 2);
    assert_eq!(state.last_flush_time, Duration::from_secs(1));
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
    assert_eq!(logger.state().length, 1);
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
    assert_eq!(logger.state().length, 2);
}

#[tokio::test]
async fn successful_flush_records_the_clock_time() {
    let (logger, transport) = logger_with_clock(
        &init("https://rubrik.test", "key"),
        SequenceClock::new(&[Duration::from_secs(10), Duration::from_secs(42)]),
        vec![ScriptedReply::ok("{}")],
    );
    assert_eq!(logger.state().last_flush_time, Duration::from_secs(10));
    append_and_maybe_flush(&logger, payload_at(0)).await;
    flush_queue(&logger).await;
    assert_eq!(logger.state().last_flush_time, Duration::from_secs(42));
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn empty_flush_sends_nothing() {
    let (logger, transport) = logger(&init("https://rubrik.test", "key"), Vec::new());
    flush_queue(&logger).await;
    assert_eq!(transport.call_count(), 0);
    assert_eq!(logger.state().length, 0);
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
    assert_eq!(logger.state().length, 2);
    flush_queue(&logger).await;
    let calls = transport.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[1].payload,
        Json::List(vec![payload_at(0), payload_at(1)])
    );
    assert_eq!(logger.state().length, 0);
}
