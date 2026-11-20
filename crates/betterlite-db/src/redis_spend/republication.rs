use serde_json::Error as JsonError;

use crate::search_tools::create::dump::safe_dumps;
use crate::spend_queues::daily::DailySpendUpdate;
use crate::spend_queues::entity::SpendUpdateTransactions;
use crate::window_queue::WindowSpendTransaction;

use super::keys::SpendBufferQueue;
use super::lists::BufferPush;
use super::publication::{RedisSpendPublisher, encode_window};

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum RepublicationError {
    #[error("an uncommitted spend cohort cannot be encoded for the redis buffer")]
    Encode(#[source] JsonError),
}

// The cohorts a failed database commit left in memory; a None is a category the caller never drained, and an empty map is falsey exactly as in the source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UncommittedCohorts {
    pub entity: Option<SpendUpdateTransactions>,
    pub daily_user: Option<DailySpendUpdate>,
    pub daily_team: Option<DailySpendUpdate>,
    pub daily_org: Option<DailySpendUpdate>,
    pub daily_end_user: Option<DailySpendUpdate>,
    pub daily_agent: Option<DailySpendUpdate>,
    pub daily_tag: Option<DailySpendUpdate>,
    pub window: Option<Vec<WindowSpendTransaction>>,
}

// restore_transactions_to_redis: push a failed commit's cohorts back so a later tick retries them. There is no memory fallback here, so a Redis failure loses them and only says so; an encoding failure is not a Redis failure and propagates.
pub async fn restore_transactions_to_redis(
    publisher: &RedisSpendPublisher,
    cohorts: UncommittedCohorts,
) -> Result<(), RepublicationError> {
    let Some(lists) = publisher.lists.as_deref() else {
        return Ok(());
    };
    let operations = encode_uncommitted(&cohorts)?;
    if operations.is_empty() {
        return Ok(());
    }
    match lists.rpush_pipeline(&operations).await {
        Ok(_lengths) => tracing::info!(
            "Spend tracking - restored {} uncommitted transaction set(s) to Redis for retry on next tick.",
            operations.len()
        ),
        Err(error) => tracing::error!(
            "Spend tracking - failed to restore uncommitted transactions to Redis. These spend updates are lost. Error: {error}"
        ),
    }
    Ok(())
}

// The restore_configs order of the source: entity, the five dailies, the daily tag the main publisher leaves out, then the window cohort.
fn encode_uncommitted(cohorts: &UncommittedCohorts) -> Result<Vec<BufferPush>, RepublicationError> {
    let mut operations = Vec::with_capacity(8);
    if let Some(entity) = &cohorts.entity {
        operations.push(BufferPush {
            key: SpendBufferQueue::Entity.names().key.to_owned(),
            values: vec![safe_dumps(entity).map_err(RepublicationError::Encode)?],
        });
    }
    let dailies = [
        (SpendBufferQueue::DailyUser, &cohorts.daily_user),
        (SpendBufferQueue::DailyTeam, &cohorts.daily_team),
        (SpendBufferQueue::DailyOrg, &cohorts.daily_org),
        (SpendBufferQueue::DailyEndUser, &cohorts.daily_end_user),
        (SpendBufferQueue::DailyAgent, &cohorts.daily_agent),
        (SpendBufferQueue::DailyTag, &cohorts.daily_tag),
    ];
    for (queue, maybe_daily) in dailies {
        let Some(daily) = maybe_daily else {
            continue;
        };
        if daily.is_empty() {
            continue;
        }
        operations.push(BufferPush {
            key: queue.names().key.to_owned(),
            values: vec![safe_dumps(daily).map_err(RepublicationError::Encode)?],
        });
    }
    let window = cohorts.window.as_deref().unwrap_or_default();
    if let Some(encoded) = encode_window(window).map_err(RepublicationError::Encode)? {
        operations.push(BufferPush {
            key: SpendBufferQueue::Window.names().key.to_owned(),
            values: vec![encoded],
        });
    }
    Ok(operations)
}
