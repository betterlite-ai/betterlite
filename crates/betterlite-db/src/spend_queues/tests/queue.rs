use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;
use tokio::time::timeout;

use super::support::capture_logs;
use crate::queues::memory::NoEnqueueObserver;
use crate::spend_queues::SpendQueueError;
use crate::spend_queues::queue::BoundedUpdateQueue;

fn sum_all(items: &[i64]) -> Result<Vec<i64>, SpendQueueError> {
    Ok(vec![items.iter().sum()])
}

fn always_fails(_items: &[i64]) -> Result<Vec<i64>, SpendQueueError> {
    Err(SpendQueueError::CostOverflow)
}

#[tokio::test]
async fn adds_keep_fifo_order_and_a_drain_takes_at_most_the_flush_count() {
    let queue = BoundedUpdateQueue::new(8, 100, 2).unwrap();
    let observer = NoEnqueueObserver;
    queue.add(1_i64, &observer).await;
    queue.add(2_i64, &observer).await;
    queue.add(3_i64, &observer).await;
    assert_eq!(queue.drain(), vec![1_i64, 2_i64]);
    assert_eq!(queue.drain(), vec![3_i64]);
    assert!(queue.drain().is_empty());
}

#[tokio::test]
async fn a_nonpositive_flush_count_drains_nothing() {
    let queue = BoundedUpdateQueue::new(8, 100, 0).unwrap();
    let observer = NoEnqueueObserver;
    queue.add(1_i64, &observer).await;
    assert!(queue.drain().is_empty());
    assert!(queue.drain().is_empty());
}

#[tokio::test]
async fn compaction_waits_for_the_threshold_and_a_threshold_of_zero_always_compacts() {
    let below = BoundedUpdateQueue::new(8, 3, 10).unwrap();
    let observer = NoEnqueueObserver;
    below.add(1_i64, &observer).await;
    below.add(2_i64, &observer).await;
    below.compact_when_full(sum_all).unwrap();
    assert_eq!(below.drain(), vec![1_i64, 2_i64]);

    let zero = BoundedUpdateQueue::new(8, 0, 10).unwrap();
    zero.compact_when_full(sum_all).unwrap();
    assert_eq!(zero.drain(), vec![0_i64]);
}

#[tokio::test]
async fn compaction_reads_at_most_the_flush_count_and_reinserts_the_aggregate() {
    let queue = BoundedUpdateQueue::new(8, 2, 2).unwrap();
    let observer = NoEnqueueObserver;
    queue.add(1_i64, &observer).await;
    queue.add(2_i64, &observer).await;
    queue.add(3_i64, &observer).await;
    queue.compact_when_full(sum_all).unwrap();
    assert_eq!(queue.drain(), vec![3_i64, 3_i64]);
}

#[tokio::test]
async fn a_failed_compaction_restores_every_drained_record() {
    let queue = BoundedUpdateQueue::new(8, 2, 10).unwrap();
    let observer = NoEnqueueObserver;
    queue.add(1_i64, &observer).await;
    queue.add(2_i64, &observer).await;
    queue.add(3_i64, &observer).await;
    assert!(matches!(
        queue.compact_when_full(always_fails),
        Err(SpendQueueError::CostOverflow)
    ));
    assert_eq!(queue.drain(), vec![1_i64, 2_i64, 3_i64]);
}

#[tokio::test]
async fn a_full_queue_blocks_a_producer_until_a_drain_and_rings_pressure() {
    let queue = Arc::new(BoundedUpdateQueue::new(1, 100, 10).unwrap());
    let observer = NoEnqueueObserver;
    queue.add("first", &observer).await;
    let mut tasks = JoinSet::new();
    let producing = Arc::clone(&queue);
    tasks.spawn(async move {
        let observer = NoEnqueueObserver;
        producing.add("second", &observer).await;
    });
    assert!(
        timeout(Duration::from_millis(20), tasks.join_next())
            .await
            .is_err()
    );
    timeout(Duration::from_millis(20), queue.pressure.notified())
        .await
        .unwrap();
    assert_eq!(queue.drain(), vec!["first"]);
    timeout(Duration::from_secs(1), tasks.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(queue.drain(), vec!["second"]);
}

#[tokio::test]
async fn a_cancelled_producer_leaves_the_queue_unchanged() {
    let queue = BoundedUpdateQueue::new(1, 100, 10).unwrap();
    let observer = NoEnqueueObserver;
    queue.add("first", &observer).await;
    assert!(
        timeout(Duration::from_millis(20), queue.add("second", &observer))
            .await
            .is_err()
    );
    assert_eq!(queue.drain(), vec!["first"]);
    queue.add("second", &observer).await;
    assert_eq!(queue.drain(), vec!["second"]);
}

#[tokio::test]
async fn concurrent_producers_lose_no_cost_when_compaction_runs() {
    let queue = Arc::new(BoundedUpdateQueue::new(256, 2, 1000).unwrap());
    let mut tasks = JoinSet::new();
    for _producer in 0..4 {
        let producing = Arc::clone(&queue);
        tasks.spawn(async move {
            let observer = NoEnqueueObserver;
            for _record in 0..25 {
                producing.add(1_i64, &observer).await;
                producing.compact_when_full(sum_all).unwrap();
            }
        });
    }
    while let Some(joined) = tasks.join_next().await {
        joined.unwrap();
    }
    let drained = queue.drain();
    assert!(!drained.is_empty());
    assert_eq!(drained.iter().sum::<i64>(), 100);
}

#[test]
fn a_capacity_of_at_least_one_is_required() {
    assert!(matches!(
        BoundedUpdateQueue::<u8>::new(0, 0, 1),
        Err(SpendQueueError::ZeroCapacity)
    ));
    assert!(matches!(
        BoundedUpdateQueue::<u8>::new(-1, 0, 1),
        Err(SpendQueueError::ZeroCapacity)
    ));
    assert!(BoundedUpdateQueue::<u8>::new(1, 0, 1).is_ok());
}

#[test]
fn a_threshold_at_capacity_warns_and_a_lower_one_does_not() {
    let logs = capture_logs();
    BoundedUpdateQueue::<u8>::new(4, 4, 10).unwrap();
    assert!(logs.contains("MAX_SIZE_IN_MEMORY_QUEUE (4) >= LITELLM_ASYNCIO_QUEUE_MAXSIZE (4)"));

    let below = capture_logs();
    BoundedUpdateQueue::<u8>::new(4, 3, 10).unwrap();
    assert!(!below.contains("Misconfigured queue thresholds"));

    let above = capture_logs();
    BoundedUpdateQueue::<u8>::new(4, 5, 10).unwrap();
    assert!(above.contains("MAX_SIZE_IN_MEMORY_QUEUE (5) >= LITELLM_ASYNCIO_QUEUE_MAXSIZE (4)"));
}
