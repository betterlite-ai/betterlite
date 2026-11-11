use super::support::{Fixture, Reply, record_at};
use crate::newrelic_metrics::drain::drain_with_retry;
use crate::newrelic_metrics::queue::{push_record, queue_snapshot};

#[tokio::test]
async fn drain_callback_interleave_keeps_a_record_appended_during_the_drain() {
    let fixture =
        Fixture::new_with_replies(&[Reply::Status(500), Reply::Status(500), Reply::Status(500)]);
    fixture.transport.push_during_send(record_at(10));
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    drain_with_retry(&logger).await;
    let queue = queue_snapshot(&logger);
    assert_eq!(queue.len(), 1);
    assert_eq!(
        queue.first().map(|record| record.model.as_str()),
        Some("gpt-10")
    );
}

#[tokio::test]
async fn drain_callback_interleave_reports_only_the_attempted_record_as_dropped() {
    let (logs, _guard) = super::support::capture_logs();
    let fixture =
        Fixture::new_with_replies(&[Reply::Status(500), Reply::Status(500), Reply::Status(500)]);
    fixture.transport.push_during_send(record_at(10));
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    drain_with_retry(&logger).await;
    assert!(logs.text().contains("dropped=1"));
}

#[tokio::test]
async fn drain_callback_interleave_does_not_warn_when_nothing_was_dropped() {
    let (logs, _guard) = super::support::capture_logs();
    let fixture =
        Fixture::new_with_replies(&[Reply::Status(202), Reply::Status(500), Reply::Status(500)]);
    fixture.transport.push_during_send(record_at(10));
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    drain_with_retry(&logger).await;
    assert_eq!(queue_snapshot(&logger).len(), 1);
    assert!(
        !logs
            .text()
            .contains("dropping records after the drain passes")
    );
}

#[tokio::test]
async fn drain_callback_interleave_delivers_an_appended_record_in_the_same_drain() {
    let fixture = Fixture::new_with_replies(&[Reply::Status(202), Reply::Status(202)]);
    fixture.transport.push_during_send(record_at(10));
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    drain_with_retry(&logger).await;
    assert_eq!(fixture.transport.requests().len(), 2);
    assert!(queue_snapshot(&logger).is_empty());
}
