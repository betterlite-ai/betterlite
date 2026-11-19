#![cfg(test)]

use std::str::FromStr as _;
use std::sync::Arc;
use std::time::Duration;

use rust_decimal::Decimal;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::queue::WindowSpendUpdateQueue;
use super::{WindowQueueError, WindowSpendTransaction, aggregate_window_spend, to_wire_payload};
use crate::spend_queues::entity::SpendAmount;

mod stamp;

const WINDOW: &str = "2026-08-01T00:00:00.000000";
const NEXT_WINDOW: &str = "2026-09-01T00:00:00.000000";

fn amount(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a test decimal")
}

fn increment(
    entity_id: &str,
    window_start: &str,
    spend: &str,
    started_at: Option<&str>,
) -> WindowSpendTransaction {
    WindowSpendTransaction {
        entity_type: String::from("key"),
        entity_id: entity_id.to_owned(),
        window_duration: String::from("30d"),
        window_start: window_start.to_owned(),
        spend: SpendAmount(amount(spend)),
        started_at: started_at.map(str::to_owned),
    }
}

fn same_window(spend: &str, started_at: Option<&str>) -> WindowSpendTransaction {
    increment("key-1", WINDOW, spend, started_at)
}

#[test]
fn increments_of_one_window_merge_and_keep_the_earliest_start() {
    let merged = aggregate_window_spend(&[
        vec![same_window("0.1", Some("2026-08-01T00:00:05.000000"))],
        vec![
            same_window("0.2", Some("2026-08-01T00:00:03.000000")),
            same_window("0.25", None),
        ],
    ])
    .expect("the windows aggregate");
    assert_eq!(
        merged,
        [increment(
            "key-1",
            WINDOW,
            "0.55",
            Some("2026-08-01T00:00:03.000000")
        )]
    );
}

#[test]
fn increments_of_different_windows_stay_distinct_and_sort_by_identity() {
    let mut other_key = increment("key-2", WINDOW, "3.0", None);
    other_key.window_duration = String::from("7d");
    let aggregated = aggregate_window_spend(&[
        vec![other_key.clone()],
        vec![increment("key-1", NEXT_WINDOW, "1.0", None)],
        vec![same_window("2.0", None)],
    ])
    .expect("the windows aggregate");
    assert_eq!(
        aggregated,
        [
            same_window("2.0", None),
            increment("key-1", NEXT_WINDOW, "1.0", None),
            other_key,
        ]
    );
}

#[test]
fn a_window_whose_increments_carry_no_start_keeps_none() {
    let aggregated = aggregate_window_spend(&[vec![same_window("1.0", None)]]).expect("one");
    assert_eq!(aggregated[0].started_at, None);
}

#[test]
fn nothing_queued_aggregates_to_nothing() {
    assert!(aggregate_window_spend(&[]).expect("no batches").is_empty());
    assert!(
        aggregate_window_spend(&[Vec::new(), Vec::new()])
            .expect("empty batches")
            .is_empty()
    );
}

#[test]
fn a_spend_sum_beyond_the_decimal_range_fails() {
    let largest = WindowSpendTransaction {
        spend: SpendAmount(Decimal::MAX),
        ..same_window("1.0", None)
    };
    let outcome = aggregate_window_spend(&[vec![largest.clone(), largest]]);
    assert!(matches!(outcome, Err(WindowQueueError::SpendOverflow)));
}

#[test]
fn the_wire_payload_always_carries_an_empty_request_id_list() {
    let payload = to_wire_payload(&same_window("0.02", Some("2026-08-01T00:00:03.000000")));
    assert_eq!(
        serde_json::to_string(&payload).expect("the payload serializes"),
        r#"{"entity_type":"key","entity_id":"key-1","window_duration":"30d","window_start":"2026-08-01T00:00:00.000000","spend":0.02,"started_at":"2026-08-01T00:00:03.000000","request_ids":[]}"#
    );
    let unstarted = to_wire_payload(&same_window("2.5", None));
    assert_eq!(unstarted.started_at, None);
    assert!(unstarted.request_ids.is_empty());
}

#[tokio::test]
async fn a_flush_merges_the_queued_increments() {
    let queue = WindowSpendUpdateQueue::new(8, 100, 1_000).expect("a queue");
    queue
        .add_update(same_window("0.25", Some(WINDOW)))
        .await
        .expect("room in the queue");
    queue
        .add_update(same_window("1.5", None))
        .await
        .expect("room in the queue");
    assert_eq!(
        queue.flush().expect("the windows aggregate"),
        [same_window("1.75", Some(WINDOW))]
    );
    assert!(queue.flush().expect("an empty flush").is_empty());
}

#[tokio::test]
async fn compaction_at_the_threshold_leaves_one_batch_and_room() {
    let queue = WindowSpendUpdateQueue::new(2, 2, 10).expect("a queue");
    queue
        .add_update(same_window("0.25", None))
        .await
        .expect("the first increment");
    queue
        .add_update(same_window("0.25", None))
        .await
        .expect("the second increment compacts");
    timeout(
        Duration::from_secs(1),
        queue.add_update(same_window("0.5", None)),
    )
    .await
    .expect("compaction freed a slot");
    assert_eq!(
        queue.flush().expect("the windows aggregate"),
        [same_window("1.0", None)]
    );
}

#[tokio::test]
async fn a_zero_threshold_compacts_every_add() {
    let queue = WindowSpendUpdateQueue::new(2, 0, 10).expect("a queue");
    timeout(
        Duration::from_secs(1),
        queue.add_update(same_window("1.0", None)),
    )
    .await
    .expect("each add compacts back to one batch")
    .expect("room in the queue");
    timeout(
        Duration::from_secs(1),
        queue.add_update(same_window("2.0", None)),
    )
    .await
    .expect("each add compacts back to one batch")
    .expect("room in the queue");
    timeout(
        Duration::from_secs(1),
        queue.add_update(same_window("0.5", None)),
    )
    .await
    .expect("each add compacts back to one batch")
    .expect("room in the queue");
    assert_eq!(
        queue.flush().expect("the windows aggregate"),
        [same_window("3.5", None)]
    );
}

#[tokio::test]
async fn a_full_queue_waits_for_a_flush() {
    let queue = WindowSpendUpdateQueue::new(1, 100, 10).expect("a queue");
    queue
        .add_update(same_window("1.0", None))
        .await
        .expect("the first increment");
    timeout(
        Duration::from_millis(20),
        queue.add_update(same_window("2.0", None)),
    )
    .await
    .unwrap_err();
    assert_eq!(
        queue.flush().expect("the first increment flushes"),
        [same_window("1.0", None)]
    );
    queue
        .add_update(same_window("2.0", None))
        .await
        .expect("the flush freed the slot");
    assert_eq!(
        queue.flush().expect("the second increment flushes"),
        [same_window("2.0", None)]
    );
}

#[tokio::test]
async fn a_flush_wakes_a_waiter_for_room() {
    let queue = Arc::new(WindowSpendUpdateQueue::new(1, 100, 10).expect("a queue"));
    queue
        .add_update(same_window("1.0", None))
        .await
        .expect("the first increment");
    let waiting = Arc::clone(&queue);
    let producer = tokio::spawn(async move { waiting.add_update(same_window("2.0", None)).await });
    tokio::task::yield_now().await;
    assert_eq!(
        queue.flush().expect("the first increment flushes"),
        [same_window("1.0", None)]
    );
    timeout(Duration::from_secs(1), producer)
        .await
        .expect("the flush wakes the producer")
        .expect("the producer queued the increment")
        .expect("the increment queued");
    assert_eq!(
        queue.flush().expect("the second increment flushes"),
        [same_window("2.0", None)]
    );
}

#[tokio::test]
async fn a_zero_flush_count_drains_nothing() {
    let queue = WindowSpendUpdateQueue::new(4, 100, 0).expect("a queue");
    queue
        .add_update(same_window("1.0", None))
        .await
        .expect("the first increment");
    assert!(queue.flush().expect("a flush takes nothing").is_empty());
    queue
        .add_update(same_window("2.0", None))
        .await
        .expect("the undrained increment leaves room");
    queue
        .add_update(same_window("3.0", None))
        .await
        .expect("the undrained increment leaves room");
    queue
        .add_update(same_window("4.0", None))
        .await
        .expect("the undrained increment leaves room");
    timeout(
        Duration::from_millis(20),
        queue.add_update(same_window("5.0", None)),
    )
    .await
    .unwrap_err();
}

#[tokio::test]
async fn a_flush_that_cannot_aggregate_keeps_every_increment() {
    let largest = WindowSpendTransaction {
        spend: SpendAmount(Decimal::MAX),
        ..same_window("1.0", None)
    };
    let queue = WindowSpendUpdateQueue::new(2, 100, 10).expect("a queue");
    queue
        .add_update(largest.clone())
        .await
        .expect("the first increment");
    queue
        .add_update(largest)
        .await
        .expect("the second increment");
    assert!(matches!(
        queue.flush(),
        Err(WindowQueueError::SpendOverflow)
    ));
    timeout(
        Duration::from_millis(20),
        queue.add_update(same_window("1.0", None)),
    )
    .await
    .unwrap_err();
}

#[tokio::test]
async fn a_compaction_that_cannot_aggregate_keeps_every_increment() {
    let largest = WindowSpendTransaction {
        spend: SpendAmount(Decimal::MAX),
        ..same_window("1.0", None)
    };
    let queue = WindowSpendUpdateQueue::new(2, 2, 10).expect("a queue");
    queue
        .add_update(largest.clone())
        .await
        .expect("the first increment");
    assert!(matches!(
        queue.add_update(largest).await,
        Err(WindowQueueError::SpendOverflow)
    ));
    timeout(
        Duration::from_millis(20),
        queue.add_update(same_window("1.0", None)),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        queue.flush(),
        Err(WindowQueueError::SpendOverflow)
    ));
}

#[tokio::test]
async fn a_flush_takes_at_most_the_flush_count_batches() {
    let queue = WindowSpendUpdateQueue::new(8, 100, 1).expect("a queue");
    queue
        .add_update(same_window("1.0", None))
        .await
        .expect("the first increment");
    queue
        .add_update(increment("key-1", NEXT_WINDOW, "2.0", None))
        .await
        .expect("the second window");
    assert_eq!(
        queue.flush().expect("one batch leaves"),
        [same_window("1.0", None)]
    );
    assert_eq!(
        queue.flush().expect("the remaining batch leaves"),
        [increment("key-1", NEXT_WINDOW, "2.0", None)]
    );
}

// one producer's share of the concurrent load: 25 increments that compaction must merge in place
async fn fill_queue(queue: Arc<WindowSpendUpdateQueue>) -> Result<(), WindowQueueError> {
    for _round in 0_i32..25_i32 {
        queue.add_update(same_window("0.25", None)).await?;
    }
    Ok(())
}

#[tokio::test]
async fn concurrent_producers_lose_no_increment_when_compaction_runs() {
    let queue = Arc::new(WindowSpendUpdateQueue::new(64, 4, 1_000).expect("a queue"));
    let mut tasks = JoinSet::new();
    tasks.spawn(fill_queue(Arc::clone(&queue)));
    tasks.spawn(fill_queue(Arc::clone(&queue)));
    tasks.spawn(fill_queue(Arc::clone(&queue)));
    tasks.spawn(fill_queue(Arc::clone(&queue)));
    while let Some(joined) = tasks.join_next().await {
        joined
            .expect("the producer task finished")
            .expect("every increment queued");
    }
    let aggregated = queue.flush().expect("the windows aggregate");
    assert_eq!(aggregated, [same_window("25.0", None)]);
}

#[test]
fn a_capacity_of_at_least_one_is_required() {
    assert!(matches!(
        WindowSpendUpdateQueue::new(0, 0, 1),
        Err(WindowQueueError::ZeroCapacity)
    ));
    assert!(matches!(
        WindowSpendUpdateQueue::new(-1, 0, 1),
        Err(WindowQueueError::ZeroCapacity)
    ));
    WindowSpendUpdateQueue::new(1, 0, 1).expect("a queue with one slot");
}
