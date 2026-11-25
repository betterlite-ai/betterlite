use std::sync::Arc;

use betterlite_db::queues::pod_lock::PodLockEventSink;
use betterlite_db::queues::pod_lock::PodLockManager;
use betterlite_db::queues::pod_lock::cache::RedisLockCache;
use betterlite_db::queues::pod_lock::owner::PodId;
use betterlite_db::redis_spend::buffer::RedisUpdateBuffer;
use betterlite_db::redis_spend::events::RedisBufferEventSink;
use betterlite_db::redis_spend::lists::{RedisSpendLists, SpendBufferLists, SpendBufferPush};
use betterlite_db::redis_spend::publication::{RedisSpendPublisher, SpendStaging};
use betterlite_db::spend_log_storage::rows::SpendLogQueue;
use betterlite_db::spend_queues::entity::SpendUpdateQueue;
use betterlite_db::spend_queues::events::SpendQueueEventSink;
use betterlite_db::tool_discovery::ToolDiscoveryQueue;
use betterlite_db::window_queue::queue::WindowSpendUpdateQueue;
use betterlite_redis::redis_cache::RedisCache;

use super::parts::DailyQueues;
use super::{SpendRuntimeError, SpendRuntimeSettings};

pub(super) struct RuntimeQueues {
    pub(super) entity: Arc<SpendUpdateQueue>,
    pub(super) daily: DailyQueues,
    pub(super) discovery: Arc<ToolDiscoveryQueue>,
    pub(super) window: Arc<WindowSpendUpdateQueue>,
    pub(super) logs: Arc<SpendLogQueue>,
}

pub(super) fn build_queues(
    settings: &SpendRuntimeSettings,
    events: &Arc<dyn SpendQueueEventSink>,
) -> Result<RuntimeQueues, SpendRuntimeError> {
    let queue = settings.queue;
    Ok(RuntimeQueues {
        entity: Arc::new(SpendUpdateQueue::new(
            queue.queue_capacity,
            queue.aggregation_threshold,
            queue.flush_count,
            Arc::clone(events),
        )?),
        daily: DailyQueues::new(&queue, events)?,
        discovery: Arc::new(ToolDiscoveryQueue::new(queue.queue_capacity)?),
        window: Arc::new(WindowSpendUpdateQueue::new(
            queue.queue_capacity,
            queue.aggregation_threshold,
            queue.flush_count,
        )?),
        logs: Arc::new(SpendLogQueue::new(settings.log_queue_bytes)),
    })
}

pub(super) fn build_redis(
    settings: &SpendRuntimeSettings,
    cache: Option<&Arc<RedisCache>>,
    events: Arc<dyn RedisBufferEventSink>,
) -> (RedisUpdateBuffer, RedisSpendPublisher) {
    let (lists, push) = redis_lists(cache);
    let buffer = RedisUpdateBuffer {
        lists,
        enabled: settings.use_redis_transaction_buffer,
        dequeue_count: settings.dequeue_count,
        events: Arc::clone(&events),
    };
    let publisher = RedisSpendPublisher {
        lists: push,
        events,
        staging: SpendStaging::default(),
    };
    (buffer, publisher)
}

fn redis_lists(
    cache: Option<&Arc<RedisCache>>,
) -> (
    Option<Arc<dyn SpendBufferLists>>,
    Option<Arc<dyn SpendBufferPush>>,
) {
    let Some(connected) = cache else {
        return (None, None);
    };
    let lists = Arc::new(RedisSpendLists {
        cache: Arc::clone(connected),
    });
    let buffer: Arc<dyn SpendBufferLists> = Arc::clone(&lists);
    let push: Arc<dyn SpendBufferPush> = lists;
    (Some(buffer), Some(push))
}

pub(super) fn build_leases(
    settings: &SpendRuntimeSettings,
    cache: Option<&Arc<RedisCache>>,
    events: Arc<dyn PodLockEventSink>,
    pod_id: PodId,
) -> Arc<PodLockManager<RedisLockCache>> {
    Arc::new(PodLockManager::new(
        cache.map(|connected| {
            Arc::new(RedisLockCache {
                cache: Arc::clone(connected),
            })
        }),
        settings.queue.default_lock_ttl_seconds,
        events,
        pod_id,
    ))
}
