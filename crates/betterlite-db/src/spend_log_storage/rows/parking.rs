use crate::redis_spend::buffer::RedisUpdateBuffer;
use crate::redis_spend::log_row::SpendLogRow;

use super::{QueuePosition, SpendLogQueue};

// REDIS_SPEND_LOGS_BUFFER_DEQUEUE_COUNT: one recovery pass moves at most this fixed number of parked rows
pub const RECOVERY_LIMIT: i64 = 1000;

// requeue_spend_logs: Redis first, then the head of the memory queue, so a failed batch flushes before newer rows
pub async fn requeue_spend_logs(
    queue: &SpendLogQueue,
    redis: &RedisUpdateBuffer,
    rows: &[SpendLogRow],
) {
    if redis.park_spend_logs(rows).await {
        return;
    }
    queue.enqueue(rows, QueuePosition::Head);
}

// recover_parked_spend_logs: an unreadable Redis buffer leaves the memory queue alone, and recovered rows go to its head
pub async fn recover_parked_spend_logs(queue: &SpendLogQueue, redis: &RedisUpdateBuffer) -> usize {
    let rows = match redis.drain_spend_logs(RECOVERY_LIMIT).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!("Spend tracking - could not read parked spend logs from Redis: {error}");
            return 0;
        }
    };
    if rows.is_empty() {
        return 0;
    }
    // The memory enqueue below is synchronous, so no await sits between the pop and the replay for a cancellation to land in: the source's re-park guard covers an await this path does not have.
    queue.enqueue(&rows, QueuePosition::Head);
    let recovered = rows.len();
    tracing::info!("Spend tracking - recovered {recovered} parked spend log rows from Redis");
    recovered
}
