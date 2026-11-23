use std::mem;
use std::time::Duration;

use futures_util::future::join_all;
use tokio::sync::{Notify, Semaphore};
use tokio::time::timeout;

use self::claim::CLAIM_BATCH;
use self::queue::BaselineQueue;
use crate::baseline::store::{AppendResult, BaselineAccounting, ProjectResult};

pub mod claim;
pub mod queue;

pub const FLUSH_BATCH: usize = 32;
pub const FLUSH_CONCURRENCY: usize = 4;
pub const FLUSH_DEADLINE: Duration = Duration::from_secs(5);

// A timed-out or cancelled flush drops its lease, and the lease puts the whole batch back after the pending records.
pub async fn flush_baseline_accounting<A>(accounting: &A, queue: &BaselineQueue, notifier: &Notify)
where
    A: BaselineAccounting,
{
    let mut lease = queue.lease(FLUSH_BATCH);
    let leased = lease.records.len();
    let slots = &Semaphore::new(FLUSH_CONCURRENCY);
    let appends = join_all(lease.records.iter().map(|queued| async move {
        let _slot = slots.acquire().await;
        accounting.append(&queued.record).await
    }));
    let Ok(outcomes) = timeout(FLUSH_DEADLINE, appends).await else {
        return;
    };
    let appended = mem::take(&mut lease.records);
    lease.records = appended
        .into_iter()
        .zip(outcomes)
        .filter(|(_, outcome)| *outcome == AppendResult::Unavailable)
        .map(|(queued, _)| queued)
        .collect();
    let more_queued = lease.more_queued;
    let completed = lease.records.len() < leased;
    drop(lease);
    if more_queued && completed {
        notifier.notify_one();
    }
    project_dirty(accounting, notifier).await;
}

// A scope whose projection fails keeps its attempted_at stamp, so the next claim waits out the retry interval.
async fn project_dirty<A>(accounting: &A, notifier: &Notify)
where
    A: BaselineAccounting,
{
    let scopes = match accounting.claim_dirty().await {
        Ok(scopes) => scopes,
        Err(error) => {
            tracing::warn!(%error, "Auto-router baseline projection will retry on a later spend flush");
            return;
        }
    };
    let slots = &Semaphore::new(FLUSH_CONCURRENCY);
    let projections = join_all(scopes.iter().map(|scope| async move {
        let _slot = slots.acquire().await;
        accounting.project(scope).await
    }));
    match timeout(FLUSH_DEADLINE, projections).await {
        Ok(outcomes) => {
            if scopes.len() == CLAIM_BATCH && outcomes.contains(&ProjectResult::Published) {
                notifier.notify_one();
            }
        }
        Err(elapsed) => {
            tracing::warn!(%elapsed, "Auto-router baseline projection will retry on a later spend flush");
        }
    }
}

mod tests;
