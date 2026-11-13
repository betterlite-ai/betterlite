use std::time::Duration;

use super::support::{Fixture, Reply, record_at};
use crate::newrelic_metrics::drain::drain_with_retry;
use crate::newrelic_metrics::queue::{push_record, queue_length};

#[tokio::test]
async fn drain_behavior_returns_after_the_first_successful_pass() {
    let fixture = Fixture::new_with_replies(&[Reply::Status(202)]);
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    drain_with_retry(&logger).await;
    assert_eq!(fixture.transport.requests().len(), 1);
    assert!(fixture.sleeper.sleeps().is_empty());
    assert_eq!(queue_length(&logger), 0);
}

#[tokio::test]
async fn drain_behavior_retries_three_passes_with_exponential_backoff() {
    let fixture =
        Fixture::new_with_replies(&[Reply::Status(500), Reply::Status(500), Reply::Status(500)]);
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    drain_with_retry(&logger).await;
    assert_eq!(fixture.transport.requests().len(), 3);
    assert_eq!(
        fixture.sleeper.sleeps(),
        [Duration::from_secs(1), Duration::from_secs(2)]
    );
    assert_eq!(queue_length(&logger), 0);
}

#[tokio::test]
async fn drain_behavior_attempts_every_chunk_of_every_pass() {
    let fixture = Fixture::new_with_replies(&[
        Reply::Status(500),
        Reply::Status(500),
        Reply::Status(500),
        Reply::Status(500),
        Reply::Status(500),
        Reply::Status(500),
    ]);
    let logger = fixture.build();
    (0_usize..250).for_each(|_record| push_record(&logger, record_at(0)));

    drain_with_retry(&logger).await;
    assert_eq!(fixture.transport.requests().len(), 6);
    assert_eq!(queue_length(&logger), 0);
}

#[tokio::test]
async fn drain_behavior_leaves_nothing_after_a_late_success() {
    let fixture = Fixture::new_with_replies(&[Reply::Status(503), Reply::Status(202)]);
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    drain_with_retry(&logger).await;
    assert_eq!(fixture.transport.requests().len(), 2);
    assert_eq!(fixture.sleeper.sleeps(), [Duration::from_secs(1)]);
    assert_eq!(queue_length(&logger), 0);
}

#[tokio::test]
async fn drain_behavior_does_nothing_on_an_empty_queue() {
    let fixture = Fixture::new();
    let logger = fixture.build();
    drain_with_retry(&logger).await;
    assert!(fixture.transport.requests().is_empty());
    assert!(fixture.sleeper.sleeps().is_empty());
}

#[tokio::test]
async fn drain_behavior_warns_about_the_dropped_survivors() {
    let (logs, _guard) = super::support::capture_logs();
    let fixture =
        Fixture::new_with_replies(&[Reply::Status(500), Reply::Status(500), Reply::Status(500)]);
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    drain_with_retry(&logger).await;
    let text = logs.text();
    assert!(text.contains("dropped=1"));
    assert!(text.contains("dropping records after the drain passes"));
}
