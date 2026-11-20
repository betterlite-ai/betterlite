use std::collections::BTreeMap;

use serde_json::Error as JsonError;
use serde_json::value::RawValue;

use crate::spend_queues::SpendQueueError;
use crate::spend_queues::daily::{DailySpendUpdate, aggregate_daily_rows};
use crate::spend_queues::entity::{SpendAmount, SpendUpdateTransactions};
use crate::window_queue::{WindowQueueError, WindowSpendTransaction, aggregate_window_spend};

use super::log_row::{SpendLogError, SpendLogRow, decode_spend_log_row};

// The bound on one buffered record: these bytes come from Redis and a decode must not allocate without limit.
pub const MAX_SPEND_BUFFER_BYTES: usize = 0x0400_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendBufferKind {
    Entity,
    Daily,
    Window,
    Log,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpendBufferRecord {
    Entity(Box<SpendUpdateTransactions>),
    Daily(DailySpendUpdate),
    Window(Vec<WindowSpendTransaction>),
    Log(SpendLogRow),
    IgnoredLogRow,
}

#[derive(Debug, thiserror::Error)]
pub enum SpendBufferError {
    #[error("a spend buffer record exceeds {MAX_SPEND_BUFFER_BYTES} bytes")]
    TooLarge,
    #[error("a spend buffer record is not valid JSON")]
    Json(#[source] JsonError),
    #[error("a spend log row cannot be decoded")]
    Log(#[from] SpendLogError),
    #[error("an entity spend batch is not a JSON object")]
    EntityShape,
    #[error("an entity spend amount cannot be read")]
    EntityValue(#[source] JsonError),
    #[error("adding entity spend overflows the decimal range")]
    EntityOverflow,
    #[error("a daily spend batch is not a JSON object")]
    DailyShape,
    #[error("a daily spend payload is not a JSON object")]
    DailyPayload(#[source] JsonError),
    #[error("a window spend batch is not a JSON array")]
    WindowShape,
    #[error("a window spend record does not have the expected fields")]
    WindowRecord(#[source] JsonError),
    #[error("aggregating daily spend records fails")]
    DailyAggregate(#[from] SpendQueueError),
    #[error("aggregating window spend records fails")]
    WindowAggregate(#[from] WindowQueueError),
}

// decode_spend_buffer: the one entry the byte fuzzer drives, with the same limit and typed failures for every kind.
pub fn decode_spend_buffer(
    kind: SpendBufferKind,
    bytes: &[u8],
) -> Result<SpendBufferRecord, SpendBufferError> {
    match kind {
        SpendBufferKind::Entity => {
            decode_entity_batch(bytes).map(|batch| SpendBufferRecord::Entity(Box::new(batch)))
        }
        SpendBufferKind::Daily => decode_daily_batch(bytes).map(SpendBufferRecord::Daily),
        SpendBufferKind::Window => decode_window_batch(bytes).map(SpendBufferRecord::Window),
        SpendBufferKind::Log => match decode_log_record(bytes)? {
            Some(row) => Ok(SpendBufferRecord::Log(row)),
            None => Ok(SpendBufferRecord::IgnoredLogRow),
        },
    }
}

// The spend_buffer fuzz target and its replay test tag their input with the low two bits of the first byte.
pub fn decode_tagged_spend_buffer(
    bytes: &[u8],
) -> Result<Option<SpendBufferRecord>, SpendBufferError> {
    let Some((tag, rest)) = bytes.split_first() else {
        return Ok(None);
    };
    let kind = match tag & 3 {
        0 => SpendBufferKind::Entity,
        1 => SpendBufferKind::Daily,
        2 => SpendBufferKind::Window,
        _ => SpendBufferKind::Log,
    };
    decode_spend_buffer(kind, rest).map(Some)
}

// DBSpendUpdateTransactions: an object of the eleven entity maps, where a map that is absent or not an object contributes nothing.
pub fn decode_entity_batch(bytes: &[u8]) -> Result<SpendUpdateTransactions, SpendBufferError> {
    let raw = parse_raw(bytes)?;
    decode_entity_object(raw.get())
}

fn decode_entity_object(text: &str) -> Result<SpendUpdateTransactions, SpendBufferError> {
    if !text.starts_with('{') {
        return Err(SpendBufferError::EntityShape);
    }
    let fields: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(text).map_err(SpendBufferError::Json)?;
    Ok(SpendUpdateTransactions {
        user_list_transactions: amount_map(&fields, "user_list_transactions")?,
        end_user_list_transactions: amount_map(&fields, "end_user_list_transactions")?,
        key_list_transactions: amount_map(&fields, "key_list_transactions")?,
        team_list_transactions: amount_map(&fields, "team_list_transactions")?,
        team_member_list_transactions: amount_map(&fields, "team_member_list_transactions")?,
        org_list_transactions: amount_map(&fields, "org_list_transactions")?,
        org_member_list_transactions: amount_map(&fields, "org_member_list_transactions")?,
        project_list_transactions: amount_map(&fields, "project_list_transactions")?,
        tag_list_transactions: amount_map(&fields, "tag_list_transactions")?,
        agent_list_transactions: amount_map(&fields, "agent_list_transactions")?,
        model_access_group_spend: amount_map(&fields, "model_access_group_list_transactions")?,
    })
}

fn amount_map(
    fields: &BTreeMap<String, Box<RawValue>>,
    name: &str,
) -> Result<BTreeMap<String, SpendAmount>, SpendBufferError> {
    let Some(raw) = fields.get(name) else {
        return Ok(BTreeMap::new());
    };
    if !raw.get().starts_with('{') {
        return Ok(BTreeMap::new());
    }
    serde_json::from_str(raw.get()).map_err(SpendBufferError::EntityValue)
}

// _accumulated_spend over all eleven entity maps: every later record adds onto the id it shares with an earlier one.
pub fn combine_entity_transactions(
    batches: &[SpendUpdateTransactions],
) -> Result<SpendUpdateTransactions, SpendBufferError> {
    let mut combined = SpendUpdateTransactions::default();
    for batch in batches {
        accumulate(
            &mut combined.user_list_transactions,
            &batch.user_list_transactions,
        )?;
        accumulate(
            &mut combined.end_user_list_transactions,
            &batch.end_user_list_transactions,
        )?;
        accumulate(
            &mut combined.key_list_transactions,
            &batch.key_list_transactions,
        )?;
        accumulate(
            &mut combined.team_list_transactions,
            &batch.team_list_transactions,
        )?;
        accumulate(
            &mut combined.team_member_list_transactions,
            &batch.team_member_list_transactions,
        )?;
        accumulate(
            &mut combined.org_list_transactions,
            &batch.org_list_transactions,
        )?;
        accumulate(
            &mut combined.org_member_list_transactions,
            &batch.org_member_list_transactions,
        )?;
        accumulate(
            &mut combined.project_list_transactions,
            &batch.project_list_transactions,
        )?;
        accumulate(
            &mut combined.tag_list_transactions,
            &batch.tag_list_transactions,
        )?;
        accumulate(
            &mut combined.agent_list_transactions,
            &batch.agent_list_transactions,
        )?;
        accumulate(
            &mut combined.model_access_group_spend,
            &batch.model_access_group_spend,
        )?;
    }
    Ok(combined)
}

fn accumulate(
    totals: &mut BTreeMap<String, SpendAmount>,
    entities: &BTreeMap<String, SpendAmount>,
) -> Result<(), SpendBufferError> {
    for (entity_id, amount) in entities {
        let total = totals.entry(entity_id.clone()).or_insert(SpendAmount::ZERO);
        total.0 = total
            .0
            .checked_add(amount.0)
            .ok_or(SpendBufferError::EntityOverflow)?;
    }
    Ok(())
}

// A daily buffer record: one JSON object of daily transaction keys, each holding its payload object.
pub fn decode_daily_batch(bytes: &[u8]) -> Result<DailySpendUpdate, SpendBufferError> {
    let raw = parse_raw(bytes)?;
    decode_daily_object(raw.get())
}

fn decode_daily_object(text: &str) -> Result<DailySpendUpdate, SpendBufferError> {
    if !text.starts_with('{') {
        return Err(SpendBufferError::DailyShape);
    }
    serde_json::from_str(text).map_err(SpendBufferError::DailyPayload)
}

// A window buffer record: one JSON array of wire payloads, `request_ids` tolerated as the rolling-deploy residue it is.
pub fn decode_window_batch(bytes: &[u8]) -> Result<Vec<WindowSpendTransaction>, SpendBufferError> {
    let raw = parse_raw(bytes)?;
    decode_window_text(raw.get())
}

fn decode_window_text(text: &str) -> Result<Vec<WindowSpendTransaction>, SpendBufferError> {
    if !text.starts_with('[') {
        return Err(SpendBufferError::WindowShape);
    }
    // A record without started_at decodes with None, and one still carrying request_ids keeps only the fields below.
    serde_json::from_str(text).map_err(SpendBufferError::WindowRecord)
}

// _decode_spend_log_row behind the byte ceiling: a JSON value that is not an object decodes to no row at all.
pub fn decode_log_record(bytes: &[u8]) -> Result<Option<SpendLogRow>, SpendBufferError> {
    if bytes.len() > MAX_SPEND_BUFFER_BYTES {
        return Err(SpendBufferError::TooLarge);
    }
    decode_spend_log_row(bytes).map_err(SpendBufferError::Log)
}

fn parse_raw(bytes: &[u8]) -> Result<&RawValue, SpendBufferError> {
    if bytes.len() > MAX_SPEND_BUFFER_BYTES {
        return Err(SpendBufferError::TooLarge);
    }
    serde_json::from_slice(bytes).map_err(SpendBufferError::Json)
}
