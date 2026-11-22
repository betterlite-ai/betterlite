#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_redis::list_operations::error::RedisListOperationsError;
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::redis_spend::buffer::RedisUpdateBuffer;
use crate::redis_spend::events::NoRedisBufferEventSink;
use crate::redis_spend::keys::REDIS_SPEND_LOGS_BUFFER_KEY;
use crate::redis_spend::lists::{
    BufferPop, BufferReply, SpendBufferListError, SpendBufferLists, SpendListFuture,
};
use crate::redis_spend::log_row::SpendLogRow;
use crate::spend_batches::spend_log_row_bytes;
use crate::spend_log_storage::tests::support::{ids, ordinary_row};

use super::parking::{RECOVERY_LIMIT, recover_parked_spend_logs, requeue_spend_logs};
use super::{QueuePosition, SpendLogQueue};

const WAIT: Duration = Duration::from_millis(50);

#[test]
fn head_replay_precedes_newly_queued_rows() {
    let queue = SpendLogQueue::new(usize::MAX);
    queue.enqueue(&[ordinary_row("new")], QueuePosition::Tail);
    queue.enqueue(&[ordinary_row("replayed")], QueuePosition::Head);
    assert_eq!(ids(&queue.dequeue(10)), ["replayed", "new"]);
}

#[test]
fn enqueue_trims_the_oldest_rows_to_the_byte_budget() {
    let queued = ordinary_row("a");
    let byte_budget = spend_log_row_bytes(&queued).saturating_mul(2);
    let queue = SpendLogQueue::new(byte_budget);
    queue.enqueue(
        &[ordinary_row("a"), ordinary_row("b"), ordinary_row("c")],
        QueuePosition::Tail,
    );
    assert_eq!(ids(&queue.dequeue(10)), ["b", "c"]);
}

#[test]
fn an_oversized_newest_row_survives_trimming_alone() {
    let queue = SpendLogQueue::new(0);
    queue.enqueue(&[ordinary_row("old")], QueuePosition::Tail);
    queue.enqueue(&[ordinary_row("new")], QueuePosition::Tail);
    assert_eq!(
        queue.queued_bytes(),
        spend_log_row_bytes(&ordinary_row("new"))
    );
    assert_eq!(ids(&queue.dequeue(10)), ["new"]);
}

#[test]
fn trimming_drops_a_replayed_batch_before_the_rows_queued_behind_it() {
    let single = spend_log_row_bytes(&ordinary_row("a"));
    let queue = SpendLogQueue::new(single.saturating_mul(2));
    queue.enqueue(&[ordinary_row("c")], QueuePosition::Tail);
    queue.enqueue(&[ordinary_row("a"), ordinary_row("b")], QueuePosition::Head);
    assert_eq!(ids(&queue.dequeue(10)), ["b", "c"]);
}

#[test]
fn dequeue_pops_the_oldest_rows_and_accounts_their_bytes() {
    let queue = SpendLogQueue::new(usize::MAX);
    queue.enqueue(
        &[ordinary_row("a"), ordinary_row("b"), ordinary_row("c")],
        QueuePosition::Tail,
    );
    let queued_bytes = queue.queued_bytes();
    assert_eq!(ids(&queue.dequeue(2)), ["a", "b"]);
    assert_eq!(
        queue.queued_bytes(),
        queued_bytes
            .saturating_sub(spend_log_row_bytes(&ordinary_row("a")))
            .saturating_sub(spend_log_row_bytes(&ordinary_row("b")))
    );
    assert_eq!(ids(&queue.dequeue(9)), ["c"]);
    assert_eq!(queue.queued_bytes(), 0);
    assert!(queue.is_empty());
    assert!(queue.dequeue(1).is_empty());
}

#[test]
fn enqueueing_nothing_leaves_the_queue_untouched() {
    let queue = SpendLogQueue::new(usize::MAX);
    queue.enqueue(&[ordinary_row("a")], QueuePosition::Tail);
    queue.enqueue(&[], QueuePosition::Head);
    assert_eq!(queue.len(), 1);
    assert_eq!(ids(&queue.dequeue(1)), ["a"]);
}

#[tokio::test]
async fn a_flush_request_retains_nothing_without_a_monitor() {
    let queue = SpendLogQueue::new(usize::MAX);
    queue.request_flush();
    let monitor = Arc::new(Notify::new());
    queue.set_flush_signal(Some(Arc::clone(&monitor)));
    assert!(timeout(WAIT, monitor.notified()).await.is_err());
}

#[tokio::test]
async fn flush_requests_coalesce_into_the_monitors_next_pass() {
    let queue = SpendLogQueue::new(usize::MAX);
    let monitor = Arc::new(Notify::new());
    queue.set_flush_signal(Some(Arc::clone(&monitor)));
    queue.request_flush();
    queue.request_flush();
    timeout(WAIT, monitor.notified()).await.unwrap();
    assert!(timeout(WAIT, monitor.notified()).await.is_err());
}

#[tokio::test]
async fn clearing_the_flush_signal_stops_retaining_requests() {
    let queue = SpendLogQueue::new(usize::MAX);
    let monitor = Arc::new(Notify::new());
    queue.set_flush_signal(Some(Arc::clone(&monitor)));
    queue.set_flush_signal(None);
    queue.request_flush();
    assert!(timeout(WAIT, monitor.notified()).await.is_err());
}

#[derive(Default)]
struct ScriptedLists {
    parked: Mutex<VecDeque<Vec<u8>>>,
    pops: Mutex<Vec<(String, Option<i64>)>>,
    pushes: Mutex<Vec<Vec<String>>>,
    fail_read: bool,
}

impl ScriptedLists {
    fn parked(rows: &[SpendLogRow]) -> Self {
        let values: VecDeque<Vec<u8>> = rows
            .iter()
            .map(|row| serde_json::to_vec(row).unwrap())
            .collect();
        Self {
            parked: Mutex::new(values),
            ..Self::default()
        }
    }

    fn unreadable() -> Self {
        Self {
            fail_read: true,
            ..Self::default()
        }
    }

    fn pops(&self) -> Vec<(String, Option<i64>)> {
        self.pops
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn pushes(&self) -> Vec<Vec<String>> {
        self.pushes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SpendBufferLists for ScriptedLists {
    fn pop<'call>(
        &'call self,
        key: &'call str,
        count: Option<i64>,
    ) -> SpendListFuture<'call, BufferReply> {
        Box::pin(async move {
            self.pops
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((key.to_owned(), count));
            if self.fail_read {
                return Err(list_failure());
            }
            let take = count.map_or(usize::MAX, |count| {
                usize::try_from(count).unwrap_or(usize::MAX)
            });
            let mut parked = self.parked.lock().unwrap_or_else(PoisonError::into_inner);
            let taken = take.min(parked.len());
            Ok(BufferReply::List(parked.drain(..taken).collect()))
        })
    }

    fn pop_pipeline<'call>(
        &'call self,
        _operations: &'call [BufferPop],
    ) -> SpendListFuture<'call, Vec<BufferReply>> {
        Box::pin(async move { Ok(Vec::new()) })
    }

    fn push_and_trim<'call>(
        &'call self,
        _key: &'call str,
        values: &'call [String],
        _max_len: i64,
    ) -> SpendListFuture<'call, i64> {
        Box::pin(async move {
            self.pushes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(values.to_vec());
            Ok(i64::try_from(values.len()).unwrap_or(i64::MAX))
        })
    }
}

const fn list_failure() -> SpendBufferListError {
    SpendBufferListError::Redis {
        operation: "async_lpop",
        source: RedisListOperationsError::UnexpectedReply {
            operation: "async_lpop",
        },
    }
}

fn buffer(lists: Option<Arc<dyn SpendBufferLists>>) -> RedisUpdateBuffer {
    RedisUpdateBuffer {
        lists,
        enabled: lists.is_some(),
        dequeue_count: RECOVERY_LIMIT,
        events: Arc::new(NoRedisBufferEventSink),
    }
}

#[tokio::test]
async fn a_replayed_batch_goes_back_to_the_head_when_redis_is_unavailable() {
    let queue = SpendLogQueue::new(usize::MAX);
    queue.enqueue(&[ordinary_row("new")], QueuePosition::Tail);
    let redis = buffer(None);
    requeue_spend_logs(&queue, &redis, &[ordinary_row("failed")]).await;
    assert_eq!(ids(&queue.dequeue(10)), ["failed", "new"]);
}

#[tokio::test]
async fn a_replayed_batch_parked_in_redis_stays_out_of_memory() {
    let queue = SpendLogQueue::new(usize::MAX);
    let lists = Arc::new(ScriptedLists::default());
    let redis = buffer(Some(Arc::clone(&lists) as Arc<dyn SpendBufferLists>));
    requeue_spend_logs(&queue, &redis, &[ordinary_row("failed")]).await;
    assert!(queue.is_empty());
    let pushes = lists.pushes();
    assert_eq!(pushes.len(), 1);
    assert_eq!(pushes.first().map(Vec::len), Some(1));
}

#[tokio::test]
async fn recovery_replays_parked_rows_before_the_memory_queue() {
    let queue = SpendLogQueue::new(usize::MAX);
    queue.enqueue(&[ordinary_row("new")], QueuePosition::Tail);
    let lists = Arc::new(ScriptedLists::parked(&[
        ordinary_row("a"),
        ordinary_row("b"),
    ]));
    let redis = buffer(Some(Arc::clone(&lists) as Arc<dyn SpendBufferLists>));
    assert_eq!(recover_parked_spend_logs(&queue, &redis).await, 2);
    assert_eq!(ids(&queue.dequeue(10)), ["a", "b", "new"]);
    assert_eq!(
        lists.pops(),
        [(REDIS_SPEND_LOGS_BUFFER_KEY.to_owned(), Some(RECOVERY_LIMIT))]
    );
}

#[tokio::test]
async fn an_unreadable_parked_buffer_leaves_the_memory_queue_alone() {
    let queue = SpendLogQueue::new(usize::MAX);
    queue.enqueue(&[ordinary_row("new")], QueuePosition::Tail);
    let lists = Arc::new(ScriptedLists::unreadable());
    let redis = buffer(Some(Arc::clone(&lists) as Arc<dyn SpendBufferLists>));
    assert_eq!(recover_parked_spend_logs(&queue, &redis).await, 0);
    assert_eq!(ids(&queue.dequeue(10)), ["new"]);
}

#[tokio::test]
async fn an_empty_parked_buffer_recovers_nothing() {
    let queue = SpendLogQueue::new(usize::MAX);
    let lists = Arc::new(ScriptedLists::default());
    let redis = buffer(Some(Arc::clone(&lists) as Arc<dyn SpendBufferLists>));
    assert_eq!(recover_parked_spend_logs(&queue, &redis).await, 0);
    assert_eq!(lists.pops().len(), 1);
}
