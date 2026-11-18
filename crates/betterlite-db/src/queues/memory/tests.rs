#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::timeout;

use super::{EnqueueObserver, MemoryQueueError, MemoryUpdateQueue, NoEnqueueObserver};
use crate::queues::test_support::captured_logs;

#[derive(Default)]
struct RecordingObserver(Mutex<Vec<usize>>);

impl EnqueueObserver for RecordingObserver {
    fn item_added(&self, queue_size: usize) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(queue_size);
    }
}

#[tokio::test]
async fn adds_updates_in_fifo_order_and_drains_them() {
    let queue = MemoryUpdateQueue::<i32>::new(4, 3, 10, Arc::new(NoEnqueueObserver)).unwrap();
    queue.add_update(1_i32).await.unwrap();
    queue.add_update(2_i32).await.unwrap();
    queue.add_update(3_i32).await.unwrap();
    assert_eq!(queue.drain_updates(), vec![1_i32, 2_i32, 3_i32]);
    assert!(queue.drain_updates().is_empty());
}

#[tokio::test]
async fn drains_at_most_the_flush_count_per_pass() {
    let queue = MemoryUpdateQueue::<i32>::new(8, 4, 2, Arc::new(NoEnqueueObserver)).unwrap();
    queue.add_update(0_i32).await.unwrap();
    queue.add_update(1_i32).await.unwrap();
    queue.add_update(2_i32).await.unwrap();
    queue.add_update(3_i32).await.unwrap();
    queue.add_update(4_i32).await.unwrap();
    assert_eq!(queue.drain_updates(), vec![0_i32, 1_i32]);
    assert_eq!(queue.drain_updates(), vec![2_i32, 3_i32]);
    assert_eq!(queue.drain_updates(), vec![4_i32]);
}

#[tokio::test]
async fn a_nonpositive_flush_count_drains_nothing() {
    let mut queue = MemoryUpdateQueue::new(4, 2, 0, Arc::new(NoEnqueueObserver)).unwrap();
    queue.add_update("first").await.unwrap();
    queue.add_update("second").await.unwrap();
    assert!(queue.drain_updates().is_empty());
    queue.flush_count = -1;
    assert!(queue.drain_updates().is_empty());
    queue.flush_count = 1;
    assert_eq!(queue.drain_updates(), vec!["first"]);
    assert_eq!(queue.drain_updates(), vec!["second"]);
}

#[test]
fn warns_when_the_threshold_cannot_trigger() {
    let (logs, _guard) = captured_logs();
    let queue = MemoryUpdateQueue::<u8>::new(4, 4, 10, Arc::new(NoEnqueueObserver)).unwrap();
    assert_eq!(queue.aggregation_threshold, 4);
    assert!(
        logs.text()
            .contains("MAX_SIZE_IN_MEMORY_QUEUE (4) >= LITELLM_ASYNCIO_QUEUE_MAXSIZE (4)")
    );
}

#[test]
fn keeps_a_threshold_below_capacity_without_warning() {
    let (logs, _guard) = captured_logs();
    let queue = MemoryUpdateQueue::<u8>::new(4, 3, 10, Arc::new(NoEnqueueObserver)).unwrap();
    assert_eq!(queue.aggregation_threshold, 3);
    assert!(!logs.text().contains("Misconfigured queue thresholds"));
}

#[tokio::test]
async fn add_update_waits_for_capacity_and_a_cancelled_add_admits_nothing() {
    let queue = MemoryUpdateQueue::new(1, 0, 10, Arc::new(NoEnqueueObserver)).unwrap();
    queue.add_update("first").await.unwrap();
    let cancelled = timeout(Duration::from_millis(20), queue.add_update("second"));
    cancelled.await.unwrap_err();
    assert_eq!(queue.drain_updates(), vec!["first"]);
    queue.add_update("second").await.unwrap();
    assert_eq!(queue.drain_updates(), vec!["second"]);
}

#[tokio::test]
async fn a_closed_queue_keeps_its_admitted_records_and_refuses_new_ones() {
    let queue = MemoryUpdateQueue::<i32>::new(2, 0, 10, Arc::new(NoEnqueueObserver)).unwrap();
    queue.add_update(7_i32).await.unwrap();
    queue.close();
    assert!(matches!(
        queue.add_update(8_i32).await,
        Err(MemoryQueueError::Closed)
    ));
    assert_eq!(queue.drain_updates(), vec![7_i32]);
}

#[tokio::test]
async fn observer_sees_the_queue_size_after_admission() {
    let observer = Arc::new(RecordingObserver::default());
    let events: Arc<dyn EnqueueObserver> = Arc::<RecordingObserver>::clone(&observer);
    let queue = MemoryUpdateQueue::new(3, 2, 10, events).unwrap();
    queue.add_update("one").await.unwrap();
    queue.add_update("two").await.unwrap();
    assert_eq!(
        observer
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        vec![1, 2]
    );
}

#[tokio::test]
async fn debug_logs_keep_the_update_body_out() {
    let (logs, _guard) = captured_logs();
    let queue = MemoryUpdateQueue::new(2, 1, 10, Arc::new(NoEnqueueObserver)).unwrap();
    queue.add_update("sk-secret-spend").await.unwrap();
    let text = logs.text();
    assert!(text.contains("Adding update to the in-memory spend queue"));
    assert!(!text.contains("sk-secret-spend"));
}

#[test]
fn rejects_zero_capacity() {
    let queue = MemoryUpdateQueue::<u8>::new(0, 0, 1, Arc::new(NoEnqueueObserver));
    assert!(matches!(queue, Err(MemoryQueueError::ZeroCapacity)));
}
