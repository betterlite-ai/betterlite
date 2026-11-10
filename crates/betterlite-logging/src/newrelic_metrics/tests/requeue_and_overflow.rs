use super::support::{Fixture, Reply, capture_logs, record_at};
use crate::newrelic_metrics::flush::async_send_batch;
use crate::newrelic_metrics::logger::NewRelicMetricsLogger;
use crate::newrelic_metrics::queue::{
    push_record, queue_length, queue_snapshot, requeue, take_front,
};

#[tokio::test]
async fn requeue_and_overflow_prepends_the_retried_batch_in_order() {
    let (logs, _guard) = capture_logs();
    let fixture = Fixture::new_with_replies(&[Reply::Status(408)]);
    fixture.transport.push_during_send(record_at(10));
    let logger = fixture.build();
    (0_usize..3).for_each(|index| push_record(&logger, record_at(index)));

    assert!(!async_send_batch(&logger).await);
    let models = models(&logger);
    assert_eq!(models, ["gpt-0", "gpt-1", "gpt-2", "gpt-10"]);
    assert!(!logs.text().contains("retry queue exceeded max_queue_size"));
}

#[tokio::test]
async fn requeue_and_overflow_drops_the_oldest_records_first() {
    let fixture = Fixture::new_with_replies(&[Reply::Status(500)]);
    fixture.transport.push_during_send(record_at(10_000));
    let logger = fixture.build();
    (0_usize..10_000).for_each(|index| push_record(&logger, record_at(index)));

    assert!(!async_send_batch(&logger).await);
    let models = models(&logger);
    assert_eq!(models.len(), 10_000);
    assert_eq!(models.first().map(String::as_str), Some("gpt-1"));
    assert_eq!(models.last().map(String::as_str), Some("gpt-10000"));
}

#[tokio::test]
async fn requeue_and_overflow_caps_at_ten_thousand_records() {
    let fixture = Fixture::new();
    let logger = fixture.build();
    (0_usize..10_000).for_each(|index| push_record(&logger, record_at(index)));

    let batch = take_front(&logger, 9_999);
    push_record(&logger, record_at(10_000));
    requeue(&logger, batch);
    assert_eq!(queue_length(&logger), 10_000);
    assert_eq!(models(&logger).first().map(String::as_str), Some("gpt-1"));
}

#[tokio::test]
async fn requeue_and_overflow_warns_when_it_drops_records() {
    let (logs, _guard) = capture_logs();
    let fixture = Fixture::new_with_replies(&[Reply::Status(500)]);
    fixture.transport.push_during_send(record_at(10_000));
    let logger = fixture.build();
    (0_usize..10_000).for_each(|index| push_record(&logger, record_at(index)));
    assert!(!async_send_batch(&logger).await);
    assert!(logs.text().contains("retry queue exceeded max_queue_size"));
    assert!(logs.text().contains("dropped=1"));
}

#[tokio::test]
async fn requeue_and_overflow_trims_the_queue_on_push_without_a_flush() {
    let (logs, _guard) = capture_logs();
    let fixture = Fixture::new();
    let logger = fixture.build();
    (0_usize..=10_000).for_each(|index| push_record(&logger, record_at(index)));

    assert_eq!(queue_length(&logger), 10_000);
    assert_eq!(models(&logger).first().map(String::as_str), Some("gpt-1"));
    assert_eq!(
        models(&logger).last().map(String::as_str),
        Some("gpt-10000")
    );
    assert!(logs.text().contains("dropped=1"));
}

fn models(logger: &NewRelicMetricsLogger) -> Vec<String> {
    queue_snapshot(logger)
        .iter()
        .map(|record| record.model.clone())
        .collect()
}
