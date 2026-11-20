use std::collections::BTreeMap;
use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Error as JsonError;

use crate::search_tools::create::dump::safe_dumps;
use crate::spend_queues::SpendQueueError;
use crate::spend_queues::daily::{DailySpendUpdate, DailySpendUpdateQueue, aggregate_daily_rows};
use crate::spend_queues::entity::{SpendUpdateQueue, SpendUpdateTransactions};
use crate::window_queue::{
    WindowQueueError, WindowSpendTransaction, WindowSpendUpdateQueue, WindowSpendWirePayload,
    aggregate_window_spend, to_wire_payload,
};

use self::restore::restore_cohorts;
use super::events::RedisBufferEventSink;
use super::keys::SpendBufferQueue;
use super::lists::{BufferPush, SpendBufferPush};

mod restore;
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum SpendPublishError {
    #[error("a spend cohort cannot be encoded for the redis buffer")]
    Encode(#[source] JsonError),
    #[error(transparent)]
    Queue(#[from] SpendQueueError),
    #[error(transparent)]
    Window(#[from] WindowQueueError),
}

// The six memory queues one scheduler tick publishes, with the window queue left out when the deployment does not run one.
#[derive(Clone, Copy)]
pub struct SpendUpdateQueues<'a> {
    pub entity: &'a SpendUpdateQueue,
    pub daily_user: &'a DailySpendUpdateQueue,
    pub daily_team: &'a DailySpendUpdateQueue,
    pub daily_org: &'a DailySpendUpdateQueue,
    pub daily_end_user: &'a DailySpendUpdateQueue,
    pub daily_agent: &'a DailySpendUpdateQueue,
    pub window: Option<&'a WindowSpendUpdateQueue>,
}

// The handle a scheduler tick publishes through: the append port, the gauge sink, and the cohorts a full memory queue could not take back.
pub struct RedisSpendPublisher {
    pub lists: Option<Arc<dyn SpendBufferPush>>,
    pub events: Arc<dyn RedisBufferEventSink>,
    pub staging: SpendStaging,
}

// A drained cohort whose queue was full on restoration. The source's asyncio queue would make the restore wait on the flusher that drained it, so the publisher keeps the cohort itself and the next tick pushes it.
#[derive(Debug, Default)]
pub struct SpendStaging {
    inner: Mutex<StagedCohorts>,
}

#[derive(Debug, Default)]
struct StagedCohorts {
    daily: BTreeMap<SpendBufferQueue, Vec<DailySpendUpdate>>,
    window: Vec<WindowSpendTransaction>,
}

impl SpendStaging {
    fn stage_daily(&self, queue: SpendBufferQueue, update: DailySpendUpdate) {
        let mut staged = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        staged.daily.entry(queue).or_default().push(update);
    }

    fn stage_window(&self, transaction: WindowSpendTransaction) {
        let mut staged = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        staged.window.push(transaction);
    }

    fn take_daily(&self, queue: SpendBufferQueue) -> Vec<DailySpendUpdate> {
        let mut staged = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        staged.daily.remove(&queue).unwrap_or_default()
    }

    fn take_window(&self) -> Vec<WindowSpendTransaction> {
        let mut staged = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        take(&mut staged.window)
    }
}

// Everything one tick drained before the push, kept whole so a failed pipeline can put it all back.
struct DrainedCohorts {
    entity: SpendUpdateTransactions,
    daily_user: DailySpendUpdate,
    daily_team: DailySpendUpdate,
    daily_org: DailySpendUpdate,
    daily_end_user: DailySpendUpdate,
    daily_agent: DailySpendUpdate,
    window: Vec<WindowSpendTransaction>,
}

// store_in_memory_spend_updates_in_redis: drain every memory queue, encode each nonempty cohort into one RPUSH pipeline, and put the whole drain back when Redis refuses the pipeline.
pub async fn store_in_memory_spend_updates_in_redis(
    publisher: &RedisSpendPublisher,
    queues: SpendUpdateQueues<'_>,
) -> Result<(), SpendPublishError> {
    let Some(lists) = publisher.lists.as_deref() else {
        tracing::debug!("redis_cache is None, skipping store_in_memory_spend_updates_in_redis");
        return Ok(());
    };
    let cohorts = drain_cohorts(publisher, queues).await?;
    let (operations, pushed_queues) = encode_cohorts(&cohorts)?;
    if operations.is_empty() {
        return Ok(());
    }
    let lengths = match lists.rpush_pipeline(&operations).await {
        Ok(lengths) => lengths,
        Err(error) => {
            tracing::error!(
                "Spend tracking - failed to push aggregated spend updates to Redis. Restoring {} transaction sets to in-memory queues for retry on next tick. Error: {error}",
                operations.len()
            );
            // A pipeline that ran some of its commands can publish a cohort and still fail, so a restored increment can double on the next tick; this is at-least-once delivery.
            restore_cohorts(publisher, queues, cohorts).await?;
            return Ok(());
        }
    };
    for (queue, queue_size) in pushed_queues.into_iter().zip(lengths) {
        publisher.events.emit_queue_size(queue, queue_size);
    }
    Ok(())
}

// The drain order of the source: entity, then the five daily queues, then the window cohort; the entity aggregate is never empty, so it always reaches the pipeline.
async fn drain_cohorts(
    publisher: &RedisSpendPublisher,
    queues: SpendUpdateQueues<'_>,
) -> Result<DrainedCohorts, SpendPublishError> {
    let entity = queues.entity.flush_aggregated_transactions()?;
    let daily_user = drain_daily(publisher, queues.daily_user, SpendBufferQueue::DailyUser).await?;
    let daily_team = drain_daily(publisher, queues.daily_team, SpendBufferQueue::DailyTeam).await?;
    let daily_org = drain_daily(publisher, queues.daily_org, SpendBufferQueue::DailyOrg).await?;
    let daily_end_user = drain_daily(
        publisher,
        queues.daily_end_user,
        SpendBufferQueue::DailyEndUser,
    )
    .await?;
    let daily_agent =
        drain_daily(publisher, queues.daily_agent, SpendBufferQueue::DailyAgent).await?;
    let window = drain_window(publisher, queues.window)?;
    Ok(DrainedCohorts {
        entity,
        daily_user,
        daily_team,
        daily_org,
        daily_end_user,
        daily_agent,
        window,
    })
}

// One daily cohort: what the queue drained now, merged behind whatever an earlier tick could not put back.
async fn drain_daily(
    publisher: &RedisSpendPublisher,
    queue: &DailySpendUpdateQueue,
    queue_name: SpendBufferQueue,
) -> Result<DailySpendUpdate, SpendPublishError> {
    merge_daily(
        queue.flush_aggregated_daily_rows().await?,
        publisher.staging.take_daily(queue_name),
    )
}

// The window cohort only exists when the deployment runs a window queue.
fn drain_window(
    publisher: &RedisSpendPublisher,
    queue: Option<&WindowSpendUpdateQueue>,
) -> Result<Vec<WindowSpendTransaction>, SpendPublishError> {
    let Some(queue) = queue else {
        return Ok(Vec::new());
    };
    let drained = queue.flush()?;
    aggregate_window_spend(&[publisher.staging.take_window(), drained])
        .map_err(SpendPublishError::from)
}

// The staged cohort came from an earlier tick, so it keeps its place ahead of what this tick drained.
fn merge_daily(
    drained: DailySpendUpdate,
    staged: Vec<DailySpendUpdate>,
) -> Result<DailySpendUpdate, SpendPublishError> {
    if staged.is_empty() {
        return Ok(drained);
    }
    let mut batches = staged;
    batches.push(drained);
    aggregate_daily_rows(&batches).map_err(SpendPublishError::from)
}

// safe_dumps over every nonempty cohort before the pipeline is attempted: the eleven entity buckets serialize as one object, window tuples wear the wire payload, and an encoding failure leaves the pipeline untouched.
fn encode_cohorts(
    cohorts: &DrainedCohorts,
) -> Result<(Vec<BufferPush>, Vec<SpendBufferQueue>), SpendPublishError> {
    let entity = safe_dumps(&cohorts.entity).map_err(SpendPublishError::Encode)?;
    let mut operations = vec![buffer_push(SpendBufferQueue::Entity, entity)];
    let mut queues = vec![SpendBufferQueue::Entity];
    for (queue, daily) in daily_cohorts(cohorts) {
        if daily.is_empty() {
            continue;
        }
        let encoded = safe_dumps(daily).map_err(SpendPublishError::Encode)?;
        operations.push(buffer_push(queue, encoded));
        queues.push(queue);
    }
    if let Some(encoded) = encode_window(&cohorts.window).map_err(SpendPublishError::Encode)? {
        operations.push(buffer_push(SpendBufferQueue::Window, encoded));
        queues.push(SpendBufferQueue::Window);
    }
    Ok((operations, queues))
}

// The five daily cohorts in the order the source pushes them.
const fn daily_cohorts(cohorts: &DrainedCohorts) -> [(SpendBufferQueue, &DailySpendUpdate); 5] {
    [
        (SpendBufferQueue::DailyUser, &cohorts.daily_user),
        (SpendBufferQueue::DailyTeam, &cohorts.daily_team),
        (SpendBufferQueue::DailyOrg, &cohorts.daily_org),
        (SpendBufferQueue::DailyEndUser, &cohorts.daily_end_user),
        (SpendBufferQueue::DailyAgent, &cohorts.daily_agent),
    ]
}

// One encoded cohort as the single-value append the source builds.
fn buffer_push(queue: SpendBufferQueue, encoded: String) -> BufferPush {
    BufferPush {
        key: queue.names().key.to_owned(),
        values: vec![encoded],
    }
}

// Window tuples go to Redis wearing B1734's wire payload; an empty cohort has nothing to append.
pub(super) fn encode_window(
    window: &[WindowSpendTransaction],
) -> Result<Option<String>, JsonError> {
    if window.is_empty() {
        return Ok(None);
    }
    let payloads: Vec<WindowSpendWirePayload> = window.iter().map(to_wire_payload).collect();
    safe_dumps(&payloads).map(Some)
}
