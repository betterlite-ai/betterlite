use std::time::Duration;

use crate::request_usage::queues::{UsageQueue, UsageQueueError};

#[test]
fn a_queue_needs_room_for_at_least_one_record() {
    assert!(matches!(
        UsageQueue::<u8>::new(0),
        Err(UsageQueueError::ZeroCapacity)
    ));
    assert!(matches!(
        UsageQueue::<u8>::new(-1),
        Err(UsageQueueError::ZeroCapacity)
    ));
}

#[tokio::test]
async fn a_full_queue_holds_the_record_until_a_drain_frees_room() {
    let queue = UsageQueue::new(1).unwrap();
    tokio::time::timeout(Duration::from_secs(1), queue.push(1_u8))
        .await
        .expect("room for the first record")
        .unwrap();
    let waiting = queue.push(2_u8);
    tokio::pin!(waiting);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), waiting.as_mut())
            .await
            .is_err()
    );
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.drain(1), vec![1_u8]);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), waiting.as_mut())
            .await
            .is_ok()
    );
    assert_eq!(queue.drain(1), vec![2_u8]);
    assert!(queue.is_empty());
}

#[tokio::test]
async fn closing_wakes_a_producer_waiting_for_room() {
    let queue = UsageQueue::new(1).unwrap();
    tokio::time::timeout(Duration::from_secs(1), queue.push(1_u8))
        .await
        .expect("room for the first record")
        .unwrap();
    let waiting = queue.push(2_u8);
    tokio::pin!(waiting);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), waiting.as_mut())
            .await
            .is_err()
    );
    assert_eq!(queue.close(), 1);
    let outcome = tokio::time::timeout(Duration::from_secs(5), waiting.as_mut()).await;
    assert!(matches!(outcome, Ok(Err(UsageQueueError::Closed))));
    assert_eq!(queue.drain(1), vec![1_u8]);
    assert_eq!(queue.close(), 0);
    assert!(matches!(
        queue.push(3_u8).await,
        Err(UsageQueueError::Closed)
    ));
}

#[tokio::test]
async fn a_drain_takes_only_the_limit_it_was_given() {
    let queue = UsageQueue::new(4).unwrap();
    for item in [1_u8, 2_u8, 3_u8] {
        tokio::time::timeout(Duration::from_secs(1), queue.push(item))
            .await
            .expect("room for the record")
            .unwrap();
    }
    assert_eq!(queue.len(), 3);
    assert!(!queue.is_empty());
    assert_eq!(queue.drain(2), vec![1_u8, 2_u8]);
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.drain(usize::MAX), vec![3_u8]);
    assert!(queue.is_empty());
}
