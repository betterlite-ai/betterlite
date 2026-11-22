use std::fmt::Display;
use std::sync::{Mutex, PoisonError};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::redis_spend::codec::decode_log_record;
use betterlite_db::redis_spend::log_row::{SpendLogRow, SpendLogValue};
use betterlite_db::spend_log_storage::batch_claim::{
    ARETRIEVE_BATCH, BatchChargeRow, BatchClaimTable, SUCCESS, SpendLogAttachment, record_spend_log,
};
use betterlite_db::spend_log_storage::postgres::SpendLogStoreError;
use betterlite_db::spend_log_storage::rows::SpendLogQueue;
use betterlite_db::spend_log_storage::{SpendLogFuture, SpendLogsWriter};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    disable_spend_logs: bool,
    database: bool,
    #[serde(default)]
    seed: Vec<JsonData>,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    payload: JsonData,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(failure)?;
        runtime.block_on(async {
            let seed = input
                .seed
                .iter()
                .map(log_row)
                .collect::<Result<Vec<_>, _>>()?;
            let table = ScriptedTable {
                rows: Mutex::new(seed),
            };
            let queue = SpendLogQueue::new(usize::MAX);
            let mut calls = Vec::with_capacity(input.calls.len());
            for call in &input.calls {
                let payload = log_row(&call.payload)?;
                let attachment = input.database.then_some(SpendLogAttachment {
                    table: &table,
                    queue: &queue,
                });
                let recorded =
                    record_spend_log(attachment, &payload, input.disable_spend_logs).await;
                calls.push(json!({
                    "recorded": recorded,
                    "queued": queue.dequeue(usize::MAX).iter().map(row_view).collect::<Vec<_>>(),
                }));
            }
            let rows = table
                .rows
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .map(row_view)
                .collect::<Vec<_>>();
            Ok(json!({"calls": calls, "rows": rows}))
        })
    }
}

struct ScriptedTable {
    rows: Mutex<Vec<SpendLogRow>>,
}

impl SpendLogsWriter for ScriptedTable {
    type Error = SpendLogStoreError;

    fn insert_rows<'call>(
        &'call self,
        rows: &'call [SpendLogRow],
    ) -> SpendLogFuture<'call, Result<usize, SpendLogStoreError>> {
        Box::pin(async move {
            let mut stored = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
            let mut inserted = 0;
            for row in rows {
                let Some(request_id) = text_field(row, "request_id") else {
                    continue;
                };
                if stored
                    .iter()
                    .any(|existing| text_field(existing, "request_id") == Some(request_id))
                {
                    continue;
                }
                stored.push(row.clone());
                inserted = inserted.saturating_add(1);
            }
            Ok(inserted)
        })
    }
}

impl BatchClaimTable for ScriptedTable {
    fn find_batch_row<'call>(
        &'call self,
        request_id: &'call str,
    ) -> SpendLogFuture<'call, Result<Option<BatchChargeRow>, SpendLogStoreError>> {
        Box::pin(async move {
            let found = self
                .rows
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .find(|candidate| text_field(candidate, "request_id") == Some(request_id))
                .map(|found| BatchChargeRow {
                    call_type: text_field(found, "call_type").map(str::to_owned),
                    status: text_field(found, "status").map(str::to_owned),
                    spend: amount_field(found, "spend")
                        .and_then(|amount| amount.to_f64())
                        .unwrap_or_default(),
                });
            Ok(found)
        })
    }

    fn take_over<'call>(
        &'call self,
        data: &'call SpendLogRow,
        request_id: &'call str,
    ) -> SpendLogFuture<'call, Result<usize, SpendLogStoreError>> {
        Box::pin(async move {
            let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
            let mut updated = 0;
            for entry in &mut *rows {
                if text_field(entry, "request_id") != Some(request_id)
                    || text_field(entry, "call_type") != Some(ARETRIEVE_BATCH)
                    || text_field(entry, "status") != Some(SUCCESS)
                    || amount_field(entry, "spend") != Some(Decimal::ZERO)
                {
                    continue;
                }
                for (field, value) in data {
                    let _replaced = entry.insert(field.clone(), value.clone());
                }
                updated = updated.saturating_add(1);
            }
            Ok(updated)
        })
    }
}

fn row_view(row: &SpendLogRow) -> Value {
    let mut view = serde_json::to_value(row).unwrap_or(Value::Null);
    if let Value::Object(fields) = &mut view
        && let Some(amount) = amount_field(row, "spend").and_then(|amount| amount.to_f64())
    {
        let _replaced = fields.insert("spend".to_owned(), Value::String(format!("{amount:.10}")));
    }
    view
}

// _decode_spend_log_row: the case hands over the payload as JSON, the same bytes the queue decodes.
fn log_row(value: &JsonData) -> Result<SpendLogRow, Box<HelperError>> {
    let bytes = serde_json::to_vec(value).map_err(failure)?;
    decode_log_record(&bytes)
        .map_err(failure)?
        .ok_or_else(|| failure("the spend log row is not an object"))
}

fn text_field<'row>(row: &'row SpendLogRow, field: &str) -> Option<&'row str> {
    match row.get(field) {
        Some(SpendLogValue::Text(text)) => Some(text),
        Some(_) | None => None,
    }
}

fn amount_field(row: &SpendLogRow, field: &str) -> Option<Decimal> {
    match row.get(field) {
        Some(SpendLogValue::Decimal(amount)) => Some(*amount),
        Some(SpendLogValue::Whole(amount)) => Some(Decimal::from(*amount)),
        Some(_) | None => None,
    }
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "BatchClaimRow".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
