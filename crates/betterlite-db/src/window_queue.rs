pub mod queue;
pub mod stamp;

mod tests;

use std::str::Utf8Error;

use chrono::ParseError;
use serde::{Deserialize, Serialize};

use crate::spend_queues::entity::SpendAmount;

#[derive(Debug, thiserror::Error)]
pub enum WindowQueueError {
    #[error("the budget window spend queue needs room for at least one update")]
    ZeroCapacity,
    #[error("a window timestamp is not an ISO-8601 date-time")]
    InvalidStamp {
        #[source]
        source: ParseError,
    },
    #[error("a window timestamp is not UTF-8")]
    InvalidStampText {
        #[source]
        source: Utf8Error,
    },
    #[error("a window timestamp falls outside the representable date range")]
    StampOutOfRange,
    #[error("summing window spend overflows the decimal range")]
    SpendOverflow,
}

// WindowSpendTransaction of litellm/proxy/db/db_transaction_queue/window_spend_update_queue.py: the start strings stay text so a record survives the JSON trip through the Redis buffer, and started_at is the earliest request start of the merged batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSpendTransaction {
    pub entity_type: String,
    pub entity_id: String,
    pub window_duration: String,
    pub window_start: String,
    pub spend: SpendAmount,
    pub started_at: Option<String>,
}

// WindowSpendWirePayload: request_ids is always empty and exists only because a worker built before the field was dropped indexes it while merging what it pops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSpendWirePayload {
    pub entity_type: String,
    pub entity_id: String,
    pub window_duration: String,
    pub window_start: String,
    pub spend: SpendAmount,
    pub started_at: Option<String>,
    pub request_ids: Vec<String>,
}

#[must_use]
pub fn to_wire_payload(transaction: &WindowSpendTransaction) -> WindowSpendWirePayload {
    WindowSpendWirePayload {
        entity_type: transaction.entity_type.clone(),
        entity_id: transaction.entity_id.clone(),
        window_duration: transaction.window_duration.clone(),
        window_start: transaction.window_start.clone(),
        spend: transaction.spend,
        started_at: transaction.started_at.clone(),
        request_ids: Vec::new(),
    }
}

// aggregate_window_spend: increments merge only when all four identity fields match, and the output keeps that tuple's order because a flush applies an older window before the roll that supersedes it.
pub fn aggregate_window_spend(
    updates: &[Vec<WindowSpendTransaction>],
) -> Result<Vec<WindowSpendTransaction>, WindowQueueError> {
    let mut ordered: Vec<&WindowSpendTransaction> = Vec::new();
    for batch in updates {
        ordered.extend(batch);
    }
    ordered.sort_by(|left, right| {
        (
            left.entity_type.as_str(),
            left.entity_id.as_str(),
            left.window_duration.as_str(),
            left.window_start.as_str(),
        )
            .cmp(&(
                right.entity_type.as_str(),
                right.entity_id.as_str(),
                right.window_duration.as_str(),
                right.window_start.as_str(),
            ))
    });
    let mut aggregated = Vec::new();
    for group in ordered.chunk_by(|left, right| {
        left.entity_type == right.entity_type
            && left.entity_id == right.entity_id
            && left.window_duration == right.window_duration
            && left.window_start == right.window_start
    }) {
        let Some((first, rest)) = group.split_first() else {
            continue;
        };
        aggregated.push(merge_window(first, rest)?);
    }
    Ok(aggregated)
}

// math.fsum keeps every increment of a window exact; the earliest non-null started_at tells the seed which spend log rows this batch already wrote.
fn merge_window(
    first: &WindowSpendTransaction,
    rest: &[&WindowSpendTransaction],
) -> Result<WindowSpendTransaction, WindowQueueError> {
    let mut spend = first.spend.0;
    let mut earliest = first.started_at.clone();
    for payload in rest {
        spend = spend
            .checked_add(payload.spend.0)
            .ok_or(WindowQueueError::SpendOverflow)?;
        if let Some(started_at) = payload.started_at.as_deref() {
            let keep = earliest
                .as_deref()
                .is_some_and(|current| current <= started_at);
            if !keep {
                earliest = Some(started_at.to_owned());
            }
        }
    }
    Ok(WindowSpendTransaction {
        entity_type: first.entity_type.clone(),
        entity_id: first.entity_id.clone(),
        window_duration: first.window_duration.clone(),
        window_start: first.window_start.clone(),
        spend: SpendAmount(spend),
        started_at: earliest,
    })
}
