use std::sync::Arc;

use betterlite_config::spend_queue::SpendQueueSettings;
use betterlite_db::model_usage::ModelUsageTransaction;
use betterlite_db::queues::pod_lock::PodLockEventSink;
use betterlite_db::queues::pod_lock::owner::PodId;
use betterlite_db::redis_spend::events::RedisBufferEventSink;
use betterlite_db::request_usage::queues::UsageQueue;
use betterlite_db::spend_log_storage::postgres::PostgresSpendLogsWriter;
use betterlite_db::spend_queues::SpendQueueError;
use betterlite_db::spend_queues::daily::DailySpendUpdateQueue;
use betterlite_db::spend_queues::events::SpendQueueEventSink;
use betterlite_db::spend_writer::SpendWriterFailureSink;
use betterlite_models::tool_spend::ToolUsageTransaction;
use betterlite_redis::redis_cache::RedisCache;

use super::pricing::SpendPricingSource;

// The six daily queues DBSpendUpdateWriter's daily helpers charge; B1733 owns their implementation.
#[derive(Clone)]
pub struct DailyQueues {
    pub user: Arc<DailySpendUpdateQueue>,
    pub end_user: Arc<DailySpendUpdateQueue>,
    pub agent: Arc<DailySpendUpdateQueue>,
    pub team: Arc<DailySpendUpdateQueue>,
    pub org: Arc<DailySpendUpdateQueue>,
    pub tag: Arc<DailySpendUpdateQueue>,
}

impl DailyQueues {
    pub(super) fn new(
        queue: &SpendQueueSettings,
        events: &Arc<dyn SpendQueueEventSink>,
    ) -> Result<Self, SpendQueueError> {
        let [user, end_user, agent, team, org, tag] = [
            daily_queue(queue, Arc::clone(events))?,
            daily_queue(queue, Arc::clone(events))?,
            daily_queue(queue, Arc::clone(events))?,
            daily_queue(queue, Arc::clone(events))?,
            daily_queue(queue, Arc::clone(events))?,
            daily_queue(queue, Arc::clone(events))?,
        ];
        Ok(Self {
            user,
            end_user,
            agent,
            team,
            org,
            tag,
        })
    }
}

fn daily_queue(
    queue: &SpendQueueSettings,
    events: Arc<dyn SpendQueueEventSink>,
) -> Result<Arc<DailySpendUpdateQueue>, SpendQueueError> {
    Ok(Arc::new(DailySpendUpdateQueue::new(
        queue.queue_capacity,
        queue.aggregation_threshold,
        queue.flush_count,
        events,
    )?))
}

// B1744 builds the usage buffers and hands them in; the runtime only reads them while it charges a request.
#[derive(Clone, Default)]
pub struct SpendUsageQueues {
    pub tool_usage: Option<Arc<UsageQueue<ToolUsageTransaction>>>,
    pub model_usage: Option<Arc<UsageQueue<ModelUsageTransaction>>>,
}

// What the composition root supplies once: the database attachment, the cache, the pricing source and every event sink.
pub struct SpendRuntimeParts {
    pub spend_logs: Option<Arc<PostgresSpendLogsWriter>>,
    pub redis: Option<Arc<RedisCache>>,
    pub pricing: Arc<dyn SpendPricingSource>,
    pub pod_id: PodId,
    pub failures: Arc<dyn SpendWriterFailureSink>,
    pub pod_lock_events: Arc<dyn PodLockEventSink>,
    pub queue_events: Arc<dyn SpendQueueEventSink>,
    pub redis_events: Arc<dyn RedisBufferEventSink>,
    pub usage: SpendUsageQueues,
}
