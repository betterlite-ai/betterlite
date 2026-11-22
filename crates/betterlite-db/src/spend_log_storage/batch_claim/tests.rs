#![cfg(test)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rstest::rstest;
use rust_decimal::Decimal;
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::redis_spend::log_row::SpendLogRow;
use crate::spend_log_storage::rows::{QueuePosition, SpendLogQueue};
use crate::spend_log_storage::sql::{insert_statement, takeover_statement};
use crate::spend_log_storage::tests::support::{
    batch_cost_row, decimal_of, ids, ordinary_row, text, text_of,
};
use crate::spend_log_storage::{SpendLogFuture, SpendLogWriteError, SpendLogsWriter};

use super::{
    ARETRIEVE_BATCH, BatchChargeRow, BatchClaimTable, SUCCESS, SpendLogAttachment,
    batch_cost_row_to_write, is_batch_cost_row, record_spend_log,
};

mod postgres;

const WAIT: Duration = Duration::from_millis(50);

#[derive(Debug, thiserror::Error)]
#[error("the scripted spend log table failed: {operation}")]
struct TableFailure {
    operation: &'static str,
}

impl SpendLogWriteError for TableFailure {
    fn is_transient(&self) -> bool {
        false
    }

    fn is_data_rejection(&self) -> bool {
        false
    }

    fn is_service_unavailable(&self) -> bool {
        false
    }

    fn is_deadlock(&self) -> bool {
        false
    }

    fn is_database(&self) -> bool {
        true
    }
}

#[derive(Default)]
struct ScriptedTable {
    rows: Mutex<Vec<SpendLogRow>>,
    failures: Mutex<BTreeSet<&'static str>>,
    take_overs: Mutex<Vec<(SpendLogRow, String)>>,
}

impl ScriptedTable {
    fn seeded(rows: &[SpendLogRow]) -> Self {
        Self {
            rows: Mutex::new(rows.to_vec()),
            failures: Mutex::new(BTreeSet::new()),
            take_overs: Mutex::new(Vec::new()),
        }
    }

    fn failing(operation: &'static str) -> Self {
        Self {
            rows: Mutex::new(Vec::new()),
            failures: Mutex::new(BTreeSet::from([operation])),
            take_overs: Mutex::new(Vec::new()),
        }
    }

    fn rows(&self) -> Vec<SpendLogRow> {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn take_overs(&self) -> Vec<(SpendLogRow, String)> {
        self.take_overs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn refuses(&self, operation: &'static str) -> bool {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(operation)
    }

    fn insert(&self, rows: &[SpendLogRow]) -> Result<usize, TableFailure> {
        if self.refuses("insert") {
            return Err(TableFailure {
                operation: "insert",
            });
        }
        let mut stored = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let mut inserted = 0_usize;
        for row in rows {
            let request_id = text_of(row, "request_id");
            if stored
                .iter()
                .any(|existing| text_of(existing, "request_id") == request_id)
            {
                continue;
            }
            stored.push(row.clone());
            inserted = inserted.saturating_add(1);
        }
        drop(stored);
        Ok(inserted)
    }

    fn find(&self, request_id: &str) -> Result<Option<BatchChargeRow>, TableFailure> {
        if self.refuses("find") {
            return Err(TableFailure { operation: "find" });
        }
        Ok(self
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|candidate| text_of(candidate, "request_id").as_deref() == Some(request_id))
            .map(|candidate| BatchChargeRow {
                call_type: text_of(candidate, "call_type"),
                status: text_of(candidate, "status"),
                spend: decimal_of(candidate, "spend").unwrap_or(Decimal::ZERO),
            }))
    }

    fn apply(&self, data: &SpendLogRow, request_id: &str) -> usize {
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let mut updated = 0_usize;
        for entry in &mut *rows {
            if text_of(entry, "request_id").as_deref() != Some(request_id)
                || text_of(entry, "call_type").as_deref() != Some(ARETRIEVE_BATCH)
                || text_of(entry, "status").as_deref() != Some(SUCCESS)
                || decimal_of(entry, "spend") != Some(Decimal::ZERO)
            {
                continue;
            }
            for (field, value) in data {
                let _replaced = entry.insert(field.clone(), value.clone());
            }
            updated = updated.saturating_add(1);
        }
        drop(rows);
        updated
    }
}

impl SpendLogsWriter for ScriptedTable {
    type Error = TableFailure;

    fn insert_rows<'call>(
        &'call self,
        rows: &'call [SpendLogRow],
    ) -> SpendLogFuture<'call, Result<usize, TableFailure>> {
        Box::pin(async move { self.insert(rows) })
    }
}

impl BatchClaimTable for ScriptedTable {
    fn find_batch_row<'call>(
        &'call self,
        request_id: &'call str,
    ) -> SpendLogFuture<'call, Result<Option<BatchChargeRow>, TableFailure>> {
        Box::pin(async move { self.find(request_id) })
    }

    fn take_over<'call>(
        &'call self,
        data: &'call SpendLogRow,
        request_id: &'call str,
    ) -> SpendLogFuture<'call, Result<usize, TableFailure>> {
        Box::pin(async move {
            if self.refuses("take_over") {
                return Err(TableFailure {
                    operation: "take_over",
                });
            }
            self.take_overs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((data.clone(), request_id.to_owned()));
            Ok(self.apply(data, request_id))
        })
    }
}

// A table whose duplicate insert skips the row and whose lookup then finds nothing to read.
struct VanishedRowTable;

impl SpendLogsWriter for VanishedRowTable {
    type Error = TableFailure;

    fn insert_rows<'call>(
        &'call self,
        _rows: &'call [SpendLogRow],
    ) -> SpendLogFuture<'call, Result<usize, TableFailure>> {
        Box::pin(async { Ok(0) })
    }
}

impl BatchClaimTable for VanishedRowTable {
    fn find_batch_row<'call>(
        &'call self,
        _request_id: &'call str,
    ) -> SpendLogFuture<'call, Result<Option<BatchChargeRow>, TableFailure>> {
        Box::pin(async { Ok(None) })
    }

    fn take_over<'call>(
        &'call self,
        _data: &'call SpendLogRow,
        _request_id: &'call str,
    ) -> SpendLogFuture<'call, Result<usize, TableFailure>> {
        Box::pin(async { Ok(0) })
    }
}

async fn record(
    table: &ScriptedTable,
    queue: &SpendLogQueue,
    payload: &SpendLogRow,
    disable_spend_logs: bool,
) -> bool {
    record_spend_log(
        Some(SpendLogAttachment { table, queue }),
        payload,
        disable_spend_logs,
    )
    .await
}

#[test]
fn only_a_successful_retrieve_charges_a_batch() {
    assert!(is_batch_cost_row(&batch_cost_row(
        "batch-1",
        SUCCESS,
        Decimal::ONE
    )));
    assert!(!is_batch_cost_row(&batch_cost_row(
        "batch-1",
        "failure",
        Decimal::ONE
    )));
    assert!(!is_batch_cost_row(&ordinary_row("request-1")));
}

#[test]
fn logging_disabled_keeps_only_the_claim_marker_fields() {
    let payload = batch_cost_row("batch-1", SUCCESS, Decimal::ONE);
    assert_eq!(batch_cost_row_to_write(&payload, false), payload);
    let reduced = batch_cost_row_to_write(&payload, true);
    assert_eq!(
        reduced.keys().cloned().collect::<Vec<_>>(),
        [
            "call_type",
            "endTime",
            "request_id",
            "spend",
            "startTime",
            "status"
        ]
    );
}

#[tokio::test]
async fn a_successful_retrieve_claims_the_batch_once() {
    let table = ScriptedTable::default();
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("batch-1", SUCCESS, Decimal::new(25, 2));
    assert!(record(&table, &queue, &payload, false).await);
    assert_eq!(ids(&table.rows()), ["batch-1"]);
    assert!(queue.is_empty());
    assert!(!record(&table, &queue, &payload, false).await);
    assert_eq!(ids(&table.rows()), ["batch-1"]);
    assert!(queue.is_empty());
    assert!(table.take_overs().is_empty());
}

#[tokio::test]
async fn a_failed_retrieve_is_an_ordinary_spend_log() {
    let table = ScriptedTable::default();
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("batch-1", "failure", Decimal::new(25, 2));
    assert!(record(&table, &queue, &payload, false).await);
    assert!(table.rows().is_empty());
    assert_eq!(ids(&queue.dequeue(10)), ["batch-1"]);
}

#[tokio::test]
async fn another_call_type_is_an_ordinary_spend_log() {
    let table = ScriptedTable::default();
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = ordinary_row("request-1");
    assert!(record(&table, &queue, &payload, false).await);
    assert!(table.rows().is_empty());
    assert_eq!(ids(&queue.dequeue(10)), ["request-1"]);
}

#[tokio::test]
async fn a_row_of_another_call_type_leaves_the_charge_without_replacing_it() {
    let table = ScriptedTable::seeded(&[ordinary_row("shared")]);
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("shared", SUCCESS, Decimal::new(40, 2));
    assert!(record(&table, &queue, &payload, false).await);
    let rows = table.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        text_of(&rows[0], "call_type").as_deref(),
        Some("acompletion")
    );
    assert_eq!(decimal_of(&rows[0], "spend"), Some(Decimal::new(25, 2)));
    assert!(queue.is_empty());
    assert!(table.take_overs().is_empty());
}

#[tokio::test]
async fn a_claim_whose_row_vanishes_still_charges() {
    let table = VanishedRowTable;
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("batch-1", SUCCESS, Decimal::new(40, 2));
    let attachment = SpendLogAttachment {
        table: &table,
        queue: &queue,
    };
    assert!(record_spend_log(Some(attachment), &payload, false).await);
    assert!(queue.is_empty());
}

#[tokio::test]
async fn one_retrieve_takes_over_the_zero_row_and_charges() {
    let table = ScriptedTable::seeded(&[batch_cost_row("batch-1", SUCCESS, Decimal::ZERO)]);
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("batch-1", SUCCESS, Decimal::new(40, 2));
    assert!(record(&table, &queue, &payload, false).await);
    assert_eq!(
        decimal_of(&table.rows()[0], "spend"),
        Some(Decimal::new(40, 2))
    );
    assert!(!record(&table, &queue, &payload, false).await);
    assert_eq!(
        decimal_of(&table.rows()[0], "spend"),
        Some(Decimal::new(40, 2))
    );
    let take_overs = table.take_overs();
    assert_eq!(take_overs.len(), 1);
    assert!(!take_overs[0].0.contains_key("request_id"));
}

#[tokio::test]
async fn a_batch_that_costs_nothing_leaves_the_zero_row_alone() {
    let table = ScriptedTable::seeded(&[batch_cost_row("batch-1", SUCCESS, Decimal::ZERO)]);
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("batch-1", SUCCESS, Decimal::ZERO);
    assert!(!record(&table, &queue, &payload, false).await);
    assert_eq!(decimal_of(&table.rows()[0], "spend"), Some(Decimal::ZERO));
    assert!(table.take_overs().is_empty());
}

#[tokio::test]
async fn a_claim_the_database_refuses_still_charges_and_queues_the_row() {
    let table = ScriptedTable::failing("insert");
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("batch-1", SUCCESS, Decimal::new(25, 2));
    assert!(record(&table, &queue, &payload, false).await);
    assert_eq!(ids(&queue.dequeue(10)), ["batch-1"]);
}

#[tokio::test]
async fn a_claim_whose_read_fails_queues_the_row_and_charges() {
    let table = ScriptedTable::seeded(&[batch_cost_row("batch-1", SUCCESS, Decimal::new(25, 2))]);
    let _added = table.failures.lock().unwrap().insert("find");
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("batch-1", SUCCESS, Decimal::new(25, 2));
    assert!(record(&table, &queue, &payload, false).await);
    assert_eq!(ids(&queue.dequeue(10)), ["batch-1"]);
}

#[tokio::test]
async fn a_failed_take_over_leaves_the_charge_to_the_next_retrieve() {
    let table = ScriptedTable::seeded(&[batch_cost_row("batch-1", SUCCESS, Decimal::ZERO)]);
    let _added = table.failures.lock().unwrap().insert("take_over");
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("batch-1", SUCCESS, Decimal::new(40, 2));
    assert!(!record(&table, &queue, &payload, false).await);
    assert_eq!(decimal_of(&table.rows()[0], "spend"), Some(Decimal::ZERO));
    assert!(queue.is_empty());
}

#[tokio::test]
async fn logging_disabled_writes_the_claim_marker_row() {
    let table = ScriptedTable::default();
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("batch-1", SUCCESS, Decimal::new(25, 2));
    assert!(record(&table, &queue, &payload, true).await);
    let rows = table.rows();
    assert_eq!(
        rows[0].keys().cloned().collect::<Vec<_>>(),
        [
            "call_type",
            "endTime",
            "request_id",
            "spend",
            "startTime",
            "status"
        ]
    );
    assert!(!record(&table, &queue, &payload, true).await);
}

#[tokio::test]
async fn a_queued_batch_row_is_reduced_when_logging_is_disabled() {
    let table = ScriptedTable::failing("insert");
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("batch-1", SUCCESS, Decimal::new(25, 2));
    assert!(record(&table, &queue, &payload, true).await);
    let queued = queue.dequeue(10);
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].keys().cloned().collect::<Vec<_>>(),
        [
            "call_type",
            "endTime",
            "request_id",
            "spend",
            "startTime",
            "status"
        ]
    );
}

#[tokio::test]
async fn a_claim_without_a_database_attaches_nothing() {
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("batch-1", SUCCESS, Decimal::new(25, 2));
    assert!(record_spend_log::<ScriptedTable>(None, &payload, false).await);
    assert!(queue.is_empty());
    assert!(record_spend_log::<ScriptedTable>(None, &payload, true).await);
}

#[tokio::test]
async fn a_claim_without_a_text_request_id_still_charges() {
    let table = ScriptedTable::default();
    let queue = SpendLogQueue::new(usize::MAX);
    let mut payload = batch_cost_row("batch-1", SUCCESS, Decimal::new(25, 2));
    let _replaced = payload.remove("request_id");
    assert!(record(&table, &queue, &payload, false).await);
    assert!(table.rows().is_empty());
    assert!(queue.is_empty());
}

#[tokio::test]
async fn a_take_over_without_a_numeric_spend_still_charges() {
    let table = ScriptedTable::seeded(&[batch_cost_row("batch-1", SUCCESS, Decimal::ZERO)]);
    let queue = SpendLogQueue::new(usize::MAX);
    let mut payload = batch_cost_row("batch-1", SUCCESS, Decimal::new(40, 2));
    let _replaced = payload.insert("spend".to_owned(), text("free"));
    assert!(record(&table, &queue, &payload, false).await);
    assert_eq!(decimal_of(&table.rows()[0], "spend"), Some(Decimal::ZERO));
    assert!(table.take_overs().is_empty());
}

#[test]
fn the_take_over_statement_matches_only_the_uncharged_success_row() {
    let mut data = batch_cost_row("batch-1", SUCCESS, Decimal::new(40, 2));
    let _replaced = data.remove("request_id");
    let _inserted = data.insert("private_note".to_owned(), text("not a column"));
    let statement = takeover_statement(&data);
    assert!(!statement.contains(r#""request_id" = src."#));
    assert!(!statement.contains(r#""api_key" = src."api_key""#));
    assert!(!statement.contains("private_note"));
    assert!(statement.contains(r#"logs."request_id" = $2"#));
    assert!(statement.contains(r#"logs."call_type" = 'aretrieve_batch'"#));
    assert!(statement.contains(r#"logs."status" = 'success'"#));
    assert!(statement.contains(r#"logs."spend" = 0.0"#));
    assert!(statement.contains(r#""updated_at" = CURRENT_TIMESTAMP"#));
}

#[tokio::test]
async fn a_responses_call_wakes_the_monitor() {
    let table = ScriptedTable::default();
    let queue = SpendLogQueue::new(usize::MAX);
    let monitor = Arc::new(Notify::new());
    queue.set_flush_signal(Some(Arc::clone(&monitor)));
    let mut payload = ordinary_row("request-1");
    let _replaced = payload.insert("call_type".to_owned(), text("responses"));
    assert!(record(&table, &queue, &payload, false).await);
    timeout(WAIT, monitor.notified()).await.unwrap();
}

#[tokio::test]
async fn an_async_responses_call_wakes_the_monitor() {
    let table = ScriptedTable::default();
    let queue = SpendLogQueue::new(usize::MAX);
    let monitor = Arc::new(Notify::new());
    queue.set_flush_signal(Some(Arc::clone(&monitor)));
    let mut payload = ordinary_row("request-1");
    let _replaced = payload.insert("call_type".to_owned(), text("aresponses"));
    assert!(record(&table, &queue, &payload, false).await);
    timeout(WAIT, monitor.notified()).await.unwrap();
}

#[tokio::test]
async fn a_completion_leaves_the_monitor_asleep() {
    let table = ScriptedTable::default();
    let queue = SpendLogQueue::new(usize::MAX);
    let monitor = Arc::new(Notify::new());
    queue.set_flush_signal(Some(Arc::clone(&monitor)));
    assert!(record(&table, &queue, &ordinary_row("request-1"), false).await);
    assert!(timeout(WAIT, monitor.notified()).await.is_err());
}

#[tokio::test]
async fn a_disabled_ordinary_log_is_not_queued() {
    let table = ScriptedTable::default();
    let queue = SpendLogQueue::new(usize::MAX);
    assert!(record(&table, &queue, &ordinary_row("request-1"), true).await);
    assert!(queue.is_empty());
    assert!(table.rows().is_empty());
}

#[rstest]
#[case::api_key("api_key", "''")]
#[case::spend("spend", "0.0")]
#[case::total_tokens("total_tokens", "0")]
#[case::prompt_tokens("prompt_tokens", "0")]
#[case::completion_tokens("completion_tokens", "0")]
#[case::model("model", "''")]
#[case::model_id("model_id", "''")]
#[case::model_group("model_group", "''")]
#[case::custom_llm_provider("custom_llm_provider", "''")]
#[case::api_base("api_base", "''")]
#[case::user("user", "''")]
#[case::cache_hit("cache_hit", "''")]
#[case::cache_key("cache_key", "''")]
#[case::metadata("metadata", "'{}'::jsonb")]
#[case::request_tags("request_tags", "'[]'::jsonb")]
#[case::messages("messages", "'{}'::jsonb")]
#[case::response("response", "'{}'::jsonb")]
#[case::proxy_server_request("proxy_server_request", "'{}'::jsonb")]
fn the_insert_statement_defaults_every_prisma_defaulted_column(
    #[case] column: &str,
    #[case] default: &str,
) {
    let expected = format!(r#"COALESCE(src."{column}", {default})"#);
    let statement = insert_statement();
    assert!(
        statement.contains(&expected),
        "{expected} is missing from {statement}"
    );
}

#[test]
fn an_explicit_head_position_still_replays_before_a_new_row() {
    let queue = SpendLogQueue::new(usize::MAX);
    queue.enqueue(&[ordinary_row("new")], QueuePosition::Tail);
    queue.enqueue(&[ordinary_row("old")], QueuePosition::Head);
    assert_eq!(ids(&queue.dequeue(10)), ["old", "new"]);
}
