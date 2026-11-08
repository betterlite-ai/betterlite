use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::rubrik_delivery::config::MAX_QUEUE_SIZE;
use betterlite_logging::rubrik_delivery::queue::{
    append_and_maybe_flush, push_payload, queue_length, queue_snapshot,
};
use betterlite_models::decisions::Json;

use super::support::{
    Environment, FixedClock, ScriptedReply, SequenceClock, build, capture, dropped_since_warning,
    environment, fill, index_of, init, integer, last_drop_warning_time, logger_with_clock,
    logger_with_environment, object,
};

fn payload_at(index: u64) -> Json {
    object(&[("index", integer(index))])
}

#[tokio::test]
async fn appending_below_the_batch_size_does_not_flush() {
    let values = environment(&[("RUBRIK_BATCH_SIZE", "3")]);
    let (logger, transport) =
        logger_with_environment(&init("https://rubrik.test", "key"), &values, Vec::new());
    append_and_maybe_flush(&logger, payload_at(0)).await;
    append_and_maybe_flush(&logger, payload_at(1)).await;
    assert_eq!(queue_length(&logger), 2);
    assert_eq!(transport.call_count(), 0);
}

#[tokio::test]
async fn reaching_the_batch_size_flushes_immediately() {
    let values = environment(&[("RUBRIK_BATCH_SIZE", "3")]);
    let (logger, transport) = logger_with_environment(
        &init("https://rubrik.test", "key"),
        &values,
        vec![ScriptedReply::ok("{}")],
    );
    append_and_maybe_flush(&logger, payload_at(0)).await;
    append_and_maybe_flush(&logger, payload_at(1)).await;
    append_and_maybe_flush(&logger, payload_at(2)).await;
    assert_eq!(queue_length(&logger), 0);
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].payload,
        Json::List(vec![payload_at(0), payload_at(1), payload_at(2)])
    );
}

#[tokio::test]
async fn a_batch_size_of_one_flushes_every_append() {
    let values = environment(&[("RUBRIK_BATCH_SIZE", "1")]);
    let (logger, transport) = logger_with_environment(
        &init("https://rubrik.test", "key"),
        &values,
        vec![ScriptedReply::ok("{}"), ScriptedReply::ok("{}")],
    );
    append_and_maybe_flush(&logger, payload_at(0)).await;
    append_and_maybe_flush(&logger, payload_at(1)).await;
    assert_eq!(queue_length(&logger), 0);
    let calls = transport.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].payload, Json::List(vec![payload_at(0)]));
    assert_eq!(calls[1].payload, Json::List(vec![payload_at(1)]));
}

#[test]
fn overflow_drops_exactly_the_oldest_entries() {
    let values = environment(&[("RUBRIK_BATCH_SIZE", "20000")]);
    let (logger, transport) =
        logger_with_environment(&init("https://rubrik.test", "key"), &values, Vec::new());
    fill(&logger, 0, 10_003);
    assert_eq!(queue_length(&logger), MAX_QUEUE_SIZE);
    assert_eq!(dropped_since_warning(&logger), 3);
    let queue = queue_snapshot(&logger);
    assert_eq!(queue.first().and_then(index_of), Some(3));
    assert_eq!(queue.last().and_then(index_of), Some(10_002));
    assert_eq!(transport.call_count(), 0);
}

#[test]
fn an_append_at_the_cap_drops_the_oldest_event() {
    let values = environment(&[("RUBRIK_BATCH_SIZE", "20000")]);
    let (logger, transport) =
        logger_with_environment(&init("https://rubrik.test", "key"), &values, Vec::new());
    fill(&logger, 0, 10_000);
    push_payload(&logger, payload_at(10_000));
    assert_eq!(queue_length(&logger), MAX_QUEUE_SIZE);
    assert_eq!(dropped_since_warning(&logger), 1);
    let queue = queue_snapshot(&logger);
    assert_eq!(queue.first().and_then(index_of), Some(1));
    assert_eq!(queue.last().and_then(index_of), Some(10_000));
    assert_eq!(transport.call_count(), 0);
}

#[test]
fn queue_overflow_warns_at_most_once_per_interval() {
    let (result, warnings) = capture(|| {
        let clock = SequenceClock::new(&[
            Duration::from_secs(0),
            Duration::from_secs(100),
            Duration::from_secs(120),
            Duration::from_secs(200),
            Duration::from_secs(210),
        ]);
        let (logger, transport) =
            logger_with_clock(&init("https://rubrik.test", "key"), clock, Vec::new());
        fill(&logger, 0, 10_000);
        fill(&logger, 10_000, 4);
        (logger, transport)
    });
    let (logger, transport) = result;
    assert_eq!(queue_length(&logger), MAX_QUEUE_SIZE);
    assert_eq!(dropped_since_warning(&logger), 1);
    assert_eq!(last_drop_warning_time(&logger), Duration::from_secs(200));
    assert_eq!(warnings.count_containing("log queue exceeded"), 2);
    let queue = queue_snapshot(&logger);
    assert_eq!(queue.first().and_then(index_of), Some(4));
    assert_eq!(queue.last().and_then(index_of), Some(10_003));
    assert_eq!(transport.call_count(), 0);
}

#[test]
fn overflow_below_the_warning_interval_accumulates_dropped_events() {
    let (result, warnings) = capture(|| {
        let clock = SequenceClock::new(&[
            Duration::from_secs(0),
            Duration::from_secs(100),
            Duration::from_secs(110),
        ]);
        let (logger, transport) =
            logger_with_clock(&init("https://rubrik.test", "key"), clock, Vec::new());
        fill(&logger, 0, 10_001);
        fill(&logger, 10_001, 1);
        (logger, transport)
    });
    let (logger, _transport) = result;
    assert_eq!(dropped_since_warning(&logger), 1);
    assert_eq!(last_drop_warning_time(&logger), Duration::from_secs(100));
    assert_eq!(warnings.count_containing("log queue exceeded"), 1);
}

#[test]
fn initialization_without_a_key_warns_before_any_overflow() {
    let mut options = init("https://rubrik.test", "unused");
    options.api_key = None;
    let (result, warnings) = capture(|| {
        build(
            &options,
            &Environment::new(),
            Arc::new(FixedClock(Duration::from_secs(1))),
            Vec::new(),
        )
    });
    let (_logger, _transport) = result.expect("initialization succeeds");
    assert_eq!(warnings.count_containing("No API key configured"), 1);
    assert_eq!(warnings.count_containing("log queue exceeded"), 0);
}
