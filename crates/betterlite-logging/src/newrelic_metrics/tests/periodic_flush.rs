use std::time::Duration;

use tokio::task::yield_now;
use tokio::time::{advance, pause};

use super::support::{Fixture, Reply, record_at};
use crate::newrelic_metrics::queue::{push_record, queue_length};

#[tokio::test]
async fn periodic_flush_sends_a_batch_every_flush_interval() {
    pause();
    let fixture = Fixture::new_with_replies(&[Reply::Status(202), Reply::Status(202)]);
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    advance(Duration::from_secs(5)).await;
    yield_now().await;
    yield_now().await;
    assert_eq!(fixture.transport.requests().len(), 1);
    push_record(&logger, record_at(1));
    advance(Duration::from_secs(5)).await;
    yield_now().await;
    yield_now().await;
    assert_eq!(fixture.transport.requests().len(), 2);
    assert_eq!(queue_length(&logger), 0);
}

#[tokio::test]
async fn periodic_flush_stops_flushing_after_stop() {
    pause();
    let fixture = Fixture::new_with_replies(&[Reply::Status(202)]);
    let logger = fixture.build();
    assert!(logger.stop());
    advance(Duration::from_secs(5)).await;
    yield_now().await;
    yield_now().await;
    logger.wait_for_final_drains().await;
    assert!(fixture.transport.requests().is_empty());
    push_record(&logger, record_at(0));
    advance(Duration::from_secs(5)).await;
    yield_now().await;
    yield_now().await;
    assert!(fixture.transport.requests().is_empty());
    assert_eq!(queue_length(&logger), 1);
}
