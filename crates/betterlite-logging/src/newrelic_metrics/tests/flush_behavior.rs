use super::support::{Fixture, Reply, capture_logs, record_at, record_count};
use crate::newrelic_metrics::flush::flush_queue;
use crate::newrelic_metrics::queue::{push_record, queue_length, queue_snapshot};

#[tokio::test]
async fn flush_behavior_splits_the_queue_into_batch_size_chunks() {
    let fixture =
        Fixture::new_with_replies(&[Reply::Status(202), Reply::Status(202), Reply::Status(202)]);
    let logger = fixture.build();
    (0_usize..450).for_each(|_record| push_record(&logger, record_at(0)));

    flush_queue(&logger).await;
    let payloads = fixture.transport.payloads();
    let counts: Vec<u64> = payloads
        .iter()
        .map(|envelopes| record_count(&envelopes[0]).to_bits())
        .collect();
    assert_eq!(
        counts,
        [200.0_f64.to_bits(), 200.0_f64.to_bits(), 50.0_f64.to_bits()]
    );
    assert_eq!(queue_length(&logger), 0);
}

#[tokio::test]
async fn flush_behavior_stops_at_the_first_failing_chunk() {
    let fixture = Fixture::new_with_replies(&[Reply::Status(202), Reply::Status(500)]);
    let logger = fixture.build();
    (0_usize..450).for_each(|_record| push_record(&logger, record_at(0)));

    flush_queue(&logger).await;
    assert_eq!(fixture.transport.requests().len(), 2);
    assert_eq!(queue_length(&logger), 250);
}

#[tokio::test]
async fn flush_behavior_leaves_records_appended_during_the_flush_for_the_next_window() {
    let fixture = Fixture::new_with_replies(&[Reply::Status(202), Reply::Status(202)]);
    fixture.transport.push_during_send(record_at(10));
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    flush_queue(&logger).await;
    assert_eq!(fixture.transport.requests().len(), 1);
    let queue = queue_snapshot(&logger);
    assert_eq!(queue.len(), 1);
    assert_eq!(
        queue.first().map(|record| record.model.as_str()),
        Some("gpt-10")
    );
    flush_queue(&logger).await;
    assert_eq!(fixture.transport.requests().len(), 2);
    assert_eq!(queue_length(&logger), 0);
}

#[tokio::test]
async fn flush_behavior_sends_nothing_for_an_empty_queue() {
    let fixture = Fixture::new();
    let logger = fixture.build();
    flush_queue(&logger).await;
    assert!(fixture.transport.requests().is_empty());
}

#[tokio::test]
async fn flush_behavior_logs_the_queued_count() {
    let (logs, _guard) = capture_logs();
    let fixture = Fixture::new();
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    push_record(&logger, record_at(1));
    flush_queue(&logger).await;
    let text = logs.text();
    assert!(text.contains("queued=2"));
    assert!(text.contains("Flushing queued records"));
}
