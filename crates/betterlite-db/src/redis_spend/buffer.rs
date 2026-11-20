use std::sync::Arc;

use crate::spend_queues::daily::{DailySpendUpdate, aggregate_daily_rows};
use crate::spend_queues::entity::SpendUpdateTransactions;
use crate::window_queue::{WindowSpendTransaction, aggregate_window_spend};

use super::codec::{
    SpendBufferError, combine_entity_transactions, decode_daily_batch, decode_entity_batch,
    decode_log_record, decode_window_batch,
};
use super::events::RedisBufferEventSink;
use super::keys::{
    REDIS_DAILY_TAG_BUFFER_KEY, REDIS_SPEND_LOGS_BUFFER_KEY, REDIS_SPEND_LOGS_MAX_ROWS,
    SpendBufferQueue,
};
use super::lists::{BufferPop, BufferReply, SpendBufferListError, SpendBufferLists};
use super::log_row::SpendLogRow;

#[derive(Debug, thiserror::Error)]
pub enum RedisSpendError {
    #[error(transparent)]
    List(#[from] SpendBufferListError),
    #[error(transparent)]
    Decode(#[from] SpendBufferError),
    #[error("the redis spend buffer returned a single value where a list belongs")]
    ScalarReply,
}

// RedisUpdateBuffer: the buffer handle itself. Transaction publishing belongs to B1738; this side parks and drains.
pub struct RedisUpdateBuffer {
    pub lists: Option<Arc<dyn SpendBufferLists>>,
    pub enabled: bool,
    pub dequeue_count: i64,
    pub events: Arc<dyn RedisBufferEventSink>,
}

// The seven-slot result of get_all_transactions_from_redis_buffer_pipeline, in its queue order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpendBufferPipeline {
    pub entity: Option<SpendUpdateTransactions>,
    pub daily_user: Option<DailySpendUpdate>,
    pub daily_team: Option<DailySpendUpdate>,
    pub daily_org: Option<DailySpendUpdate>,
    pub daily_end_user: Option<DailySpendUpdate>,
    pub daily_agent: Option<DailySpendUpdate>,
    pub window: Option<Vec<WindowSpendTransaction>>,
}

impl RedisUpdateBuffer {
    // store_spend_logs_in_redis: only a parked row can be drained later, so every failure reports the caller's memory fallback.
    pub async fn park_spend_logs(&self, rows: &[SpendLogRow]) -> bool {
        let count = rows.len();
        if count == 0 || !self.enabled {
            return false;
        }
        let Some(lists) = self.lists.as_deref() else {
            return false;
        };
        let Some(values) = encode_rows(rows) else {
            return false;
        };
        let trimmed = lists
            .push_and_trim(
                REDIS_SPEND_LOGS_BUFFER_KEY,
                &values,
                REDIS_SPEND_LOGS_MAX_ROWS,
            )
            .await;
        match trimmed {
            Ok(buffer_size) => {
                let dropped = buffer_size.saturating_sub(REDIS_SPEND_LOGS_MAX_ROWS);
                if dropped > 0 {
                    tracing::error!(
                        "Spend tracking - Redis spend log buffer is at its {REDIS_SPEND_LOGS_MAX_ROWS} row cap; dropped the {dropped} oldest spend logs"
                    );
                }
            }
            Err(error) => {
                tracing::error!(
                    "Spend tracking - failed to park {count} spend log rows in Redis. Error: {error}"
                );
                return false;
            }
        }
        tracing::info!("Spend tracking - parked {count} spend log rows in Redis for a later flush");
        true
    }

    // get_spend_logs_from_redis_buffer: a scalar or a list reply both drain, a decoded nonobject is dropped and bad JSON propagates.
    pub async fn drain_spend_logs(&self, limit: i64) -> Result<Vec<SpendLogRow>, RedisSpendError> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        let Some(lists) = self.lists.as_deref() else {
            return Ok(Vec::new());
        };
        let popped = lists.pop(REDIS_SPEND_LOGS_BUFFER_KEY, Some(limit)).await?;
        let mut rows = Vec::new();
        for encoded in popped.items() {
            rows.extend(decode_log_record(&encoded)?);
        }
        Ok(rows)
    }

    // get_all_transactions_from_redis_buffer_pipeline: one destructive pop per queue, positional, short replies padded with None.
    pub async fn drain_pipeline(&self) -> Result<SpendBufferPipeline, RedisSpendError> {
        let Some(lists) = self.lists.as_deref() else {
            return Ok(SpendBufferPipeline::default());
        };
        let operations: Vec<BufferPop> = SpendBufferQueue::PIPELINE
            .iter()
            .map(|queue| BufferPop {
                key: queue.key().to_owned(),
                count: Some(self.dequeue_count),
            })
            .collect();
        let mut entries = lists.pop_pipeline(&operations).await?;
        entries.truncate(SpendBufferQueue::PIPELINE.len());
        entries.resize(SpendBufferQueue::PIPELINE.len(), BufferReply::Missing);
        let mut slots = entries.into_iter();
        let entity = slots.next().unwrap_or(BufferReply::Missing);
        let daily_user = slots.next().unwrap_or(BufferReply::Missing);
        let daily_team = slots.next().unwrap_or(BufferReply::Missing);
        let daily_org = slots.next().unwrap_or(BufferReply::Missing);
        let daily_end_user = slots.next().unwrap_or(BufferReply::Missing);
        let daily_agent = slots.next().unwrap_or(BufferReply::Missing);
        let window = slots.next().unwrap_or(BufferReply::Missing);
        Ok(SpendBufferPipeline {
            entity: entity_slot(entity)?,
            daily_user: daily_slot(daily_user)?,
            daily_team: daily_slot(daily_team)?,
            daily_org: daily_slot(daily_org)?,
            daily_end_user: daily_slot(daily_end_user)?,
            daily_agent: daily_slot(daily_agent)?,
            window: window_slot(window)?,
        })
    }

    // get_all_daily_tag_spend_update_transactions_from_redis_buffer: the pipeline leaves daily tags out, so they have their own reader.
    pub async fn drain_daily_tags(&self) -> Result<Option<DailySpendUpdate>, RedisSpendError> {
        let Some(lists) = self.lists.as_deref() else {
            return Ok(None);
        };
        let popped = lists
            .pop(REDIS_DAILY_TAG_BUFFER_KEY, Some(self.dequeue_count))
            .await?;
        daily_slot(popped)
    }
}

// _encode_spend_log_row over the whole batch: one failure keeps every row in memory, as the tuple build in the source does.
#[cfg_attr(test, mutants::skip)]
fn encode_rows(rows: &[SpendLogRow]) -> Option<Vec<String>> {
    let mut values = Vec::with_capacity(rows.len());
    for row in rows {
        match serde_json::to_string(row) {
            Ok(encoded) => values.push(encoded),
            Err(error) => {
                tracing::error!(
                    "Spend tracking - failed to park {} spend log rows in Redis. Error: {error}",
                    rows.len()
                );
                return None;
            }
        }
    }
    Some(values)
}

// _parse_list_of_transactions: one JSON string or a list of them, and the list keeps its order for the merge.
fn entity_slot(reply: BufferReply) -> Result<Option<SpendUpdateTransactions>, SpendBufferError> {
    let items = reply.items();
    if items.is_empty() {
        return Ok(None);
    }
    let mut batches = Vec::with_capacity(items.len());
    for item in &items {
        batches.push(decode_entity_batch(item)?);
    }
    combine_entity_transactions(&batches).map(Some)
}

fn daily_slot(reply: BufferReply) -> Result<Option<DailySpendUpdate>, RedisSpendError> {
    match reply {
        BufferReply::Missing => Ok(None),
        BufferReply::Scalar(_) => Err(RedisSpendError::ScalarReply),
        BufferReply::List(items) => aggregate_daily_items(items)
            .map(Some)
            .map_err(RedisSpendError::from),
    }
}

fn window_slot(reply: BufferReply) -> Result<Option<Vec<WindowSpendTransaction>>, RedisSpendError> {
    match reply {
        BufferReply::Missing => Ok(None),
        BufferReply::Scalar(_) => Err(RedisSpendError::ScalarReply),
        BufferReply::List(items) => aggregate_window_items(items),
    }
}

fn aggregate_daily_items(items: Vec<Vec<u8>>) -> Result<DailySpendUpdate, SpendBufferError> {
    let mut records = Vec::with_capacity(items.len());
    for item in &items {
        records.push(decode_daily_batch(item)?);
    }
    aggregate_daily_rows(&records).map_err(SpendBufferError::from)
}

fn aggregate_window_items(
    items: Vec<Vec<u8>>,
) -> Result<Option<Vec<WindowSpendTransaction>>, RedisSpendError> {
    let mut batches = Vec::with_capacity(items.len());
    for item in &items {
        batches.push(decode_window_batch(item)?);
    }
    aggregate_window_spend(&batches)
        .map(Some)
        .map_err(|source| RedisSpendError::from(SpendBufferError::from(source)))
}
