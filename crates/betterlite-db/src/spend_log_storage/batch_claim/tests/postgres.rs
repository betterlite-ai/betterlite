#![cfg(test)]

use std::slice::from_ref;

use rust_decimal::Decimal;

use crate::spend_log_storage::batch_claim::{
    ARETRIEVE_BATCH, BatchClaimTable as _, SUCCESS, SpendLogAttachment, batch_cost_row_to_write,
    record_spend_log,
};
use crate::spend_log_storage::postgres::{PostgresSpendLogsWriter, insert_spend_log_rows};
use crate::spend_log_storage::rows::SpendLogQueue;
use crate::spend_log_storage::tests::support::{SpendLogCase, batch_cost_row, ordinary_row};

fn writer(case: &SpendLogCase) -> PostgresSpendLogsWriter {
    PostgresSpendLogsWriter {
        store: case.store.clone(),
    }
}

fn charged(rows: &[(String, String, String, Decimal)], request_id: &str) -> Option<Decimal> {
    rows.iter()
        .find(|(id, _, _, _)| id == request_id)
        .map(|(_, _, _, spend)| *spend)
}

#[tokio::test]
async fn a_duplicate_insert_is_skipped_and_never_replaced() {
    let case = SpendLogCase::new().await;
    let row = batch_cost_row("batch-1", SUCCESS, Decimal::new(25, 2));
    let first = insert_spend_log_rows(&case.store, from_ref(&row))
        .await
        .expect("the first statement");
    let second = insert_spend_log_rows(&case.store, from_ref(&row))
        .await
        .expect("the duplicate statement");
    assert_eq!((first, second), (1, 0));
    let rows = case.rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, ARETRIEVE_BATCH);
    assert_eq!(rows[0].2, SUCCESS);
    case.close().await;
}

#[tokio::test]
async fn only_one_of_two_retrieves_charges_the_batch() {
    let case = SpendLogCase::new().await;
    let table = writer(&case);
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("batch-1", SUCCESS, Decimal::new(25, 2));
    let attachment = || {
        Some(SpendLogAttachment {
            table: &table,
            queue: &queue,
        })
    };
    let (first, second) = tokio::join!(
        record_spend_log(attachment(), &payload, false),
        record_spend_log(attachment(), &payload, false),
    );
    assert_ne!(first, second);
    let rows = case.rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(charged(&rows, "batch-1"), Some(Decimal::new(25, 2)));
    assert!(queue.is_empty());
    case.close().await;
}

#[tokio::test]
async fn a_zero_row_takeover_charges_once_under_a_race() {
    let case = SpendLogCase::new().await;
    let zero = batch_cost_row("batch-1", SUCCESS, Decimal::ZERO);
    insert_spend_log_rows(&case.store, from_ref(&zero))
        .await
        .expect("the zero row");
    let table = writer(&case);
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("batch-1", SUCCESS, Decimal::new(50, 2));
    let attachment = || {
        Some(SpendLogAttachment {
            table: &table,
            queue: &queue,
        })
    };
    let (first, second) = tokio::join!(
        record_spend_log(attachment(), &payload, false),
        record_spend_log(attachment(), &payload, false),
    );
    assert_ne!(first, second);
    let rows = case.rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(charged(&rows, "batch-1"), Some(Decimal::new(50, 2)));
    case.close().await;
}

#[tokio::test]
async fn a_take_over_statement_updates_the_zero_row_only_once() {
    let case = SpendLogCase::new().await;
    let zero = batch_cost_row("batch-1", SUCCESS, Decimal::ZERO);
    insert_spend_log_rows(&case.store, from_ref(&zero))
        .await
        .expect("the zero row");
    let table = writer(&case);
    let mut data = batch_cost_row("batch-1", SUCCESS, Decimal::new(50, 2));
    let _replaced = data.remove("request_id");
    let first = table
        .take_over(&data, "batch-1")
        .await
        .expect("the first takeover");
    let second = table
        .take_over(&data, "batch-1")
        .await
        .expect("the repeated takeover");
    assert_eq!((first, second), (1, 0));
    case.close().await;
}

#[tokio::test]
async fn a_row_of_another_call_type_keeps_its_own_charge() {
    let case = SpendLogCase::new().await;
    insert_spend_log_rows(&case.store, from_ref(&ordinary_row("shared")))
        .await
        .expect("the ordinary row");
    let table = writer(&case);
    let queue = SpendLogQueue::new(usize::MAX);
    let payload = batch_cost_row("shared", SUCCESS, Decimal::new(40, 2));
    assert!(
        record_spend_log(
            Some(SpendLogAttachment {
                table: &table,
                queue: &queue,
            }),
            &payload,
            false,
        )
        .await
    );
    let rows = case.rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, "acompletion");
    assert_eq!(charged(&rows, "shared"), Some(Decimal::new(25, 2)));
    assert!(queue.is_empty());
    case.close().await;
}

#[tokio::test]
async fn a_marker_row_keeps_the_schema_default_in_its_text_columns() {
    let case = SpendLogCase::new().await;
    let payload = batch_cost_row("batch-marker-1", SUCCESS, Decimal::new(50, 2));
    let marker = batch_cost_row_to_write(&payload, true);
    let inserted = insert_spend_log_rows(&case.store, from_ref(&marker))
        .await
        .expect("the marker row");
    assert_eq!(inserted, 1);
    let found = case
        .client
        .query_one(
            r#"SELECT "model_id" = '' AND "model_group" = '' AND "custom_llm_provider" = '' AND "api_base" = '' AND "user" = '' AND "cache_hit" = '' AND "cache_key" = '' AS "defaulted" FROM "LiteLLM_SpendLogs" WHERE "request_id" = $1"#,
            &[&"batch-marker-1"],
        )
        .await
        .expect("the marker row read back");
    let defaulted = found.get::<_, Option<bool>>("defaulted");
    assert_eq!(defaulted, Some(true));
    case.close().await;
}
