use std::slice::from_ref;

use serde_json::Error as JsonError;

use crate::search_tools::create::dump::safe_dumps;
use crate::spend_queues::SpendQueueError;
use crate::spend_queues::daily::DailySpendUpdateQueue;

use super::keys::SpendBufferQueue;
use super::publication::RedisSpendPublisher;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum TagPublishError {
    #[error("the daily tag cohort cannot be encoded for the redis buffer")]
    Encode(#[source] JsonError),
    #[error(transparent)]
    Queue(#[from] SpendQueueError),
}

// store_in_memory_daily_tag_spend_updates_in_redis, which the source splits over _store_transactions_in_redis: the tag scheduler drains its queue before anything asks Redis, so a deployment without a Redis client loses the drained tags; a failed push only logs and never puts the drain back.
pub async fn store_daily_tag_spend_updates(
    publisher: &RedisSpendPublisher,
    queue: &DailySpendUpdateQueue,
) -> Result<(), TagPublishError> {
    let transactions = queue.flush_aggregated_daily_rows().await?;
    if transactions.is_empty() {
        return Ok(());
    }
    let encoded = safe_dumps(&transactions).map_err(TagPublishError::Encode)?;
    let Some(lists) = publisher.lists.as_deref() else {
        return Ok(());
    };
    let key = SpendBufferQueue::DailyTag.names().key;
    match lists.rpush(key, from_ref(&encoded)).await {
        Ok(queue_size) => {
            tracing::debug!(
                "Spend tracking - pushed spend updates to Redis buffer. redis_key={key}, buffer_size={queue_size}"
            );
            publisher
                .events
                .emit_queue_size(SpendBufferQueue::DailyTag, queue_size);
        }
        Err(error) => tracing::error!(
            "Spend tracking - failed to push spend updates to Redis (redis_key={key}). Error: {error}"
        ),
    }
    Ok(())
}
