use std::fmt::Display;
use std::slice::from_ref;

use rust_decimal::Decimal;

use crate::redis_spend::log_row::{SpendLogRow, SpendLogValue};

use super::rows::{QueuePosition, SpendLogQueue};
use super::{SpendLogFuture, SpendLogsWriter};

mod tests;

// CallTypes.aretrieve_batch: the only call whose successful row charges a batch
pub const ARETRIEVE_BATCH: &str = "aretrieve_batch";
pub const SUCCESS: &str = "success";

// RESPONSES_SESSION_CALL_TYPES: a responses call wakes the monitor so its row is written before the next lookup
const RESPONSES_SESSION_CALL_TYPES: [&str; 2] = ["responses", "aresponses"];

// _BATCH_COST_CLAIM_FIELDS: with spend logs off a batch still writes one row, reduced to what tells its retrieves apart
const BATCH_COST_CLAIM_FIELDS: [&str; 6] = [
    "request_id",
    "call_type",
    "spend",
    "startTime",
    "endTime",
    "status",
];

// The fields spend_logs.find_unique reads back for the claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchChargeRow {
    pub call_type: Option<String>,
    pub status: Option<String>,
    pub spend: Decimal,
}

// SpendLogsRepository's create_many, find_unique and update_many over one batch's cost row.
pub trait BatchClaimTable: SpendLogsWriter {
    fn find_batch_row<'call>(
        &'call self,
        request_id: &'call str,
    ) -> SpendLogFuture<'call, Result<Option<BatchChargeRow>, Self::Error>>;

    fn take_over<'call>(
        &'call self,
        data: &'call SpendLogRow,
        request_id: &'call str,
    ) -> SpendLogFuture<'call, Result<usize, Self::Error>>;
}

// The prisma_client one request-time write needs: the table the claim reads and the queue an unclaimed row waits in.
pub struct SpendLogAttachment<'source, Table: ?Sized> {
    pub table: &'source Table,
    pub queue: &'source SpendLogQueue,
}

// _is_batch_cost_row: only a retrieve that succeeded charges, so a failed one or any other call type is an ordinary spend log.
#[must_use]
pub fn is_batch_cost_row(payload: &SpendLogRow) -> bool {
    text_field(payload, "call_type") == Some(ARETRIEVE_BATCH)
        && text_field(payload, "status") == Some(SUCCESS)
}

// _batch_cost_row_to_write: a proxy with spend logs off still needs the row to charge the batch once, but only the claim's fields.
#[must_use]
pub fn batch_cost_row_to_write(payload: &SpendLogRow, disable_spend_logs: bool) -> SpendLogRow {
    if !disable_spend_logs {
        return payload.clone();
    }
    payload
        .iter()
        .filter(|(field, _)| BATCH_COST_CLAIM_FIELDS.contains(&field.as_str()))
        .map(|(field, value)| (field.clone(), value.clone()))
        .collect()
}

// _record_spend_log: answers whether this request's cost still belongs to its key, team and user updates.
pub async fn record_spend_log<Table>(
    attachment: Option<SpendLogAttachment<'_, Table>>,
    payload: &SpendLogRow,
    disable_spend_logs: bool,
) -> bool
where
    Table: BatchClaimTable + ?Sized,
{
    let Some(SpendLogAttachment { table, queue }) = attachment else {
        return true;
    };
    if is_batch_cost_row(payload) {
        return claim_batch_cost_spend_log(table, queue, payload, disable_spend_logs).await;
    }
    if !disable_spend_logs {
        insert_spend_log_to_db(queue, payload);
    }
    true
}

// _insert_spend_log_to_db: a row with a database attachment joins the tail of the queue, and a Responses row also wakes the monitor.
pub fn insert_spend_log_to_db(queue: &SpendLogQueue, payload: &SpendLogRow) {
    tracing::debug!(
        "Writing spend log to db - request_id: {:?}, spend: {:?}",
        text_field(payload, "request_id"),
        spend_field(payload),
    );
    queue.enqueue(from_ref(payload), QueuePosition::Tail);
    if text_field(payload, "call_type")
        .is_some_and(|call_type| RESPONSES_SESSION_CALL_TYPES.contains(&call_type))
    {
        queue.request_flush();
    }
}

// _claim_batch_cost_spend_log: the insert that lands first charges the batch, and every later retrieve reads its charge and stops.
pub async fn claim_batch_cost_spend_log<Table>(
    table: &Table,
    queue: &SpendLogQueue,
    payload: &SpendLogRow,
    disable_spend_logs: bool,
) -> bool
where
    Table: BatchClaimTable + ?Sized,
{
    let Some(request_id) = text_field(payload, "request_id") else {
        tracing::warn!(
            "Could not claim a spend row for a batch's cost: the row has no text request_id"
        );
        return true;
    };
    let row = batch_cost_row_to_write(payload, disable_spend_logs);
    let found = match table.insert_rows(from_ref(&row)).await {
        Ok(1) => return true,
        Ok(_) => table.find_batch_row(request_id).await,
        Err(error) => Err(error),
    };
    if let Err(error) = &found {
        queue_after_claim_failure(queue, request_id, &row, error);
        return true;
    }
    let Some(existing) = found.ok().flatten() else {
        tracing::warn!(
            "Spend row {request_id} belongs to a None request, so this batch's cost is charged without a row of its own"
        );
        return true;
    };
    if existing.call_type.as_deref() != Some(ARETRIEVE_BATCH)
        || existing.status.as_deref() != Some(SUCCESS)
    {
        let owner = existing.call_type.as_deref().unwrap_or("None");
        tracing::warn!(
            "Spend row {request_id} belongs to a {owner} request, so this batch's cost is charged without a row of its own"
        );
        return true;
    }
    if existing.spend > Decimal::ZERO {
        tracing::debug!("Cost tracking skipped: spend row {request_id} already charged this batch");
        return false;
    }
    take_over_uncharged_row(table, payload, request_id, &row).await
}

// _take_over_uncharged_batch_cost_row: the zero row an older proxy left behind still needs the charge, and only one retrieve may write it.
async fn take_over_uncharged_row<Table>(
    table: &Table,
    payload: &SpendLogRow,
    request_id: &str,
    row: &SpendLogRow,
) -> bool
where
    Table: BatchClaimTable + ?Sized,
{
    let Some(spend) = spend_field(payload) else {
        tracing::warn!(
            "Could not take over spend row {request_id}: the row's spend is not a number"
        );
        return true;
    };
    if spend <= Decimal::ZERO {
        tracing::debug!(
            "Cost tracking skipped: this batch costs nothing and spend row {request_id} says so"
        );
        return false;
    }
    let mut data = row.clone();
    let _replaced = data.remove("request_id");
    match table.take_over(&data, request_id).await {
        Ok(0) => {
            tracing::debug!(
                "Cost tracking skipped: spend row {request_id} already charged this batch"
            );
            false
        }
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(
                "Could not take over spend row {request_id}, leaving this batch's cost to the next retrieve: {error}"
            );
            false
        }
    }
}

// The insert and the read share one except in the source, and either one leaves the row to the next flush while the charge stands.
fn queue_after_claim_failure<Failure>(
    queue: &SpendLogQueue,
    request_id: &str,
    row: &SpendLogRow,
    error: &Failure,
) where
    Failure: Display,
{
    tracing::warn!(
        "Could not claim spend row {request_id} for a batch's cost, queueing it: {error}"
    );
    insert_spend_log_to_db(queue, row);
}

fn text_field<'row>(row: &'row SpendLogRow, field: &str) -> Option<&'row str> {
    match row.get(field) {
        Some(SpendLogValue::Text(text)) => Some(text),
        Some(_) | None => None,
    }
}

fn spend_field(row: &SpendLogRow) -> Option<Decimal> {
    match row.get("spend") {
        Some(SpendLogValue::Decimal(amount)) => Some(*amount),
        Some(SpendLogValue::Whole(amount)) => Some(Decimal::from(*amount)),
        Some(_) | None => None,
    }
}
