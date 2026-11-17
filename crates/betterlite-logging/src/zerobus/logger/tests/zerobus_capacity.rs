use std::sync::Arc;

use crate::zerobus::logger::ZerobusLogger;
use crate::zerobus::logger::delivery::RowIngest;

use super::support::{
    Buffer, FakeIngest, as_row_ingest, deps, expected_ids, log_event, params, queued_ids, settle,
    subscribe, until,
};

#[tokio::test]
async fn zerobus_capacity_drops_the_arriving_row_at_the_cap_during_a_flush() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let gate = ingest.block();
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let mut configured = deps(params(2, 3600, false), Some(client));
    configured.max_queue_size = Some(3);
    let logger = ZerobusLogger::new(configured).expect("the logger builds");
    let buffer = Buffer::default();
    let guard = subscribe(&buffer);
    log_event(&logger, "a").await;
    log_event(&logger, "b").await;
    assert!(
        until(|| ingest.inserted() == 1).await,
        "the batch flush starts"
    );
    log_event(&logger, "c").await;
    log_event(&logger, "d").await;
    log_event(&logger, "e").await;
    assert_eq!(queued_ids(&logger), expected_ids(&["c"]));
    ingest.unblock();
    gate.add_permits(1);
    settle(&logger).await;
    drop(guard);
    assert_eq!(ingest.inserted(), 1);
    assert_eq!(ingest.ids(), expected_ids(&["a", "b"]));
    assert!(
        buffer.contents().contains("dropped a row"),
        "{}",
        buffer.contents()
    );
}

#[tokio::test]
async fn zerobus_capacity_queues_an_arrival_below_the_cap_during_a_flush() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let gate = ingest.block();
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let mut configured = deps(params(1, 3600, false), Some(client));
    configured.max_queue_size = Some(3);
    let logger = ZerobusLogger::new(configured).expect("the logger builds");
    log_event(&logger, "a").await;
    assert!(
        until(|| ingest.inserted() == 1).await,
        "the batch flush starts"
    );
    log_event(&logger, "b").await;
    assert_eq!(queued_ids(&logger), expected_ids(&["b"]));
    ingest.unblock();
    gate.add_permits(1);
    settle(&logger).await;
    assert_eq!(queued_ids(&logger), expected_ids(&["b"]));
}

#[tokio::test]
async fn zerobus_capacity_queues_again_after_the_flush() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let gate = ingest.block();
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let mut configured = deps(params(2, 3600, false), Some(client));
    configured.max_queue_size = Some(3);
    let logger = ZerobusLogger::new(configured).expect("the logger builds");
    log_event(&logger, "a").await;
    log_event(&logger, "b").await;
    assert!(
        until(|| ingest.inserted() == 1).await,
        "the batch flush starts"
    );
    log_event(&logger, "c").await;
    log_event(&logger, "d").await;
    assert_eq!(queued_ids(&logger), expected_ids(&["c"]));
    ingest.unblock();
    gate.add_permits(1);
    settle(&logger).await;
    log_event(&logger, "e").await;
    settle(&logger).await;
    assert_eq!(ingest.inserted(), 2);
    assert_eq!(ingest.ids(), expected_ids(&["a", "b", "c", "e"]));
    assert!(queued_ids(&logger).is_empty());
}
