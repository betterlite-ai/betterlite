mod fields;

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::Notify;
use tokio_util::task::TaskTracker;

use self::fields::{add_counter, add_money};
use super::SpendQueueError;
use super::events::{SpendQueueEvent, SpendQueueEventSink, SpendQueueService};
use super::interrupted::{CommitProgress, SettleCompletion};
use super::queue::BoundedUpdateQueue;
use crate::daily_spend_sql::entities::SpendRow;
use crate::queues::memory::EnqueueObserver;

// One queued entry of the daily buffer: the transactional key of the row and its payload as litellm/proxy/_types.py declares it, kept as it arrived so an absent metric stays absent.
pub type DailySpendUpdate = BTreeMap<String, SpendRow>;

const REQUIRED_COUNTER_FIELDS: [&str; 5] = [
    "prompt_tokens",
    "completion_tokens",
    "api_requests",
    "successful_requests",
    "failed_requests",
];
const OPTIONAL_COUNTER_FIELDS: [&str; 5] = [
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
    "compression_saved_tokens",
    "total_response_time_ms",
    "timed_requests",
];
const REQUIRED_MONEY_FIELDS: [&str; 1] = ["spend"];
const OPTIONAL_MONEY_FIELDS: [&str; 4] = [
    "compression_savings_spend",
    "prompt_caching_savings_spend",
    "gateway_injected_caching_savings_spend",
    "autorouter_savings_spend",
];

// get_aggregated_daily_spend_update_transactions: one payload per daily transaction key, the first payload kept whole and every later one added onto it.
pub fn aggregate_daily_spend_update_transactions(
    updates: &[DailySpendUpdate],
) -> Result<DailySpendUpdate, SpendQueueError> {
    let mut aggregated = DailySpendUpdate::new();
    for update in updates {
        for (key, payload) in update {
            match aggregated.entry(key.clone()) {
                Entry::Occupied(mut occupied) => merge_daily_row(occupied.get_mut(), payload)?,
                Entry::Vacant(vacant) => {
                    vacant.insert(payload.clone());
                }
            }
        }
    }
    Ok(aggregated)
}

// The six required counters of the source are summed as integers, the five optional ones read an absent or falsey incoming value as zero, and an existing null keeps the source's invalid addition.
fn merge_daily_row(existing: &mut SpendRow, incoming: &SpendRow) -> Result<(), SpendQueueError> {
    for field in REQUIRED_COUNTER_FIELDS {
        add_counter(existing, incoming, field, true)?;
    }
    for field in OPTIONAL_COUNTER_FIELDS {
        add_counter(existing, incoming, field, false)?;
    }
    for field in REQUIRED_MONEY_FIELDS {
        add_money(existing, incoming, field, true)?;
    }
    for field in OPTIONAL_MONEY_FIELDS {
        add_money(existing, incoming, field, false)?;
    }
    Ok(())
}

pub struct DailySpendUpdateQueue {
    queue: BoundedUpdateQueue<DailySpendUpdate>,
    sink: Arc<dyn SpendQueueEventSink>,
    service_tasks: TaskTracker,
    commits: TaskTracker,
    commit_progress: Arc<CommitProgress>,
}

impl DailySpendUpdateQueue {
    pub fn new(
        queue_capacity: i64,
        aggregation_threshold: i64,
        flush_count: i64,
        sink: Arc<dyn SpendQueueEventSink>,
    ) -> Result<Self, SpendQueueError> {
        let queue = BoundedUpdateQueue::new(queue_capacity, aggregation_threshold, flush_count)?;
        Ok(Self {
            queue,
            sink,
            service_tasks: TaskTracker::new(),
            commits: TaskTracker::new(),
            commit_progress: Arc::new(CommitProgress {
                pending: AtomicUsize::new(0),
                idle: Notify::new(),
            }),
        })
    }

    pub async fn add_update(&self, update: DailySpendUpdate) -> Result<(), SpendQueueError> {
        // The source logs the whole update here; betterlite's log line keeps spend bodies out of every log.
        tracing::debug!("Adding update to the in-memory daily spend queue");
        self.queue.add(update, self).await;
        self.queue.compact_when_full(aggregate_queue_updates)
    }

    // track_interrupted_commit: the settlement runs on the queue's own task set, so shutdown waits for it and a flush never misses it.
    pub fn track_interrupted_commit<Fut>(&self, settle: Fut)
    where
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.commit_progress.pending.fetch_add(1, Ordering::SeqCst);
        let completion = SettleCompletion {
            progress: Arc::clone(&self.commit_progress),
        };
        self.commits.spawn(async move {
            let _completion = completion;
            settle.await;
        });
    }

    // settle_interrupted_commits: waits until every settlement registered so far has ended, including ones registered while it waits.
    pub async fn settle_interrupted_commits(&self) {
        loop {
            let idle = self.commit_progress.idle.notified();
            tokio::pin!(idle);
            idle.as_mut().enable();
            if self.commit_progress.pending.load(Ordering::SeqCst) == 0 {
                return;
            }
            idle.await;
        }
    }

    // flush_and_get_aggregated_daily_spend_update_transactions: settlements run first and their requeued rows join this flush.
    pub async fn flush_and_get_aggregated_daily_spend_update_transactions(
        &self,
    ) -> Result<DailySpendUpdate, SpendQueueError> {
        let mut updates = self.settle_before_flush().await;
        updates.extend(self.queue.drain());
        if !updates.is_empty() {
            tracing::info!(
                items = updates.len(),
                "Spend tracking - flushed daily spend update items from in-memory queue"
            );
        }
        aggregate_daily_spend_update_transactions(&updates)
    }

    // A settlement that needs room while the queue is full drains the queue into this batch instead of deadlocking against the flush waiting for it.
    async fn settle_before_flush(&self) -> Vec<DailySpendUpdate> {
        let mut relieved = Vec::new();
        let settle = self.settle_interrupted_commits();
        tokio::pin!(settle);
        loop {
            tokio::select! {
                () = &mut settle => return relieved,
                () = self.queue.pressure.notified() => relieved.extend(self.queue.drain()),
            }
        }
    }

    // Callers flush before shutting down, so a settlement that adds a row still finds room in the queue.
    pub async fn shutdown(&self) {
        self.service_tasks.close();
        self.commits.close();
        self.commits.wait().await;
        self.service_tasks.wait().await;
    }
}

impl EnqueueObserver for DailySpendUpdateQueue {
    fn item_added(&self, queue_size: usize) {
        let event = SpendQueueEvent {
            service: SpendQueueService::InMemoryDailySpendUpdateQueue,
            queue_size,
        };
        let sink = Arc::clone(&self.sink);
        self.service_tasks.spawn(async move { sink.emit(event) });
    }
}

fn aggregate_queue_updates(
    updates: &[DailySpendUpdate],
) -> Result<Vec<DailySpendUpdate>, SpendQueueError> {
    let aggregated = aggregate_daily_spend_update_transactions(updates)?;
    Ok(vec![aggregated])
}
