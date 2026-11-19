#![cfg(test)]

use rust_decimal::Decimal;

use crate::database_faults::ErrorType;
use crate::identity_budget_reset::phase::IdentityPhase;
use crate::identity_budget_reset::rows::{DueIdentity, read_due};
use crate::identity_budget_reset::tests::support::{
    finish, identifiers, moment, provision, schema,
};
use crate::postgres_store::session::PostgresStore;

fn row<'due>(due: &'due [DueIdentity], identifier: &str) -> &'due DueIdentity {
    due.iter().find(|row| row.identifier == identifier).unwrap()
}

#[tokio::test]
async fn key_reads_return_only_rows_due_for_a_reset() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, budget_duration, budget_reset_at) VALUES ('due', 10, '1d', '2025-12-31 00:00:00'), ('no-duration', 10, NULL, '2025-12-31 00:00:00'), ('future', 10, '1d', '2026-06-01 00:00:00'), ('never-reset', 10, '1d', NULL);",
        )
        .await
        .unwrap();
    let due = read_due(&store, IdentityPhase::Keys, moment(0), 500)
        .await
        .unwrap();
    assert_eq!(identifiers(&due), vec!["due".to_owned()]);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn key_expiry_filter_skips_expired_keys_and_keeps_live_and_unlimited_ones() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, expires, budget_duration, budget_reset_at) VALUES ('expired', 10, '2025-12-31 00:00:00', '1d', '2025-12-30 00:00:00'), ('live', 10, '2026-06-01 00:00:00', '1d', '2025-12-30 00:00:00'), ('unlimited', 10, NULL, '1d', '2025-12-30 00:00:00');",
        )
        .await
        .unwrap();
    let due = read_due(&store, IdentityPhase::Keys, moment(0), 500)
        .await
        .unwrap();
    assert_eq!(
        identifiers(&due),
        vec!["live".to_owned(), "unlimited".to_owned()]
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn key_expiry_filter_treats_a_key_expiring_now_as_expired() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, expires, budget_duration, budget_reset_at) VALUES ('expiring-now', 10, '2026-01-01 00:00:00', '1d', '2025-12-30 00:00:00');",
        )
        .await
        .unwrap();
    let due = read_due(&store, IdentityPhase::Keys, moment(0), 500)
        .await
        .unwrap();
    assert!(due.is_empty());
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn key_reads_stop_at_the_chunk_limit() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, budget_duration, budget_reset_at) VALUES ('a', 10, '1d', '2025-12-30 00:00:00'), ('b', 10, '1d', '2025-12-30 00:00:00'), ('c', 10, '1d', '2025-12-30 00:00:00');",
        )
        .await
        .unwrap();
    let due = read_due(&store, IdentityPhase::Keys, moment(0), 2)
        .await
        .unwrap();
    assert_eq!(due.len(), 2);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn key_reads_convert_missing_and_non_finite_numbers_to_zero() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, max_budget, budget_duration, budget_reset_at) VALUES ('null', NULL, NULL, '1d', '2025-12-30 00:00:00'), ('nan', 'NaN', 'NaN', '1d', '2025-12-30 00:00:00'), ('infinite', 'Infinity', '-Infinity', '1d', '2025-12-30 00:00:00'), ('finite', 12.5, 10.5, '1d', '2025-12-30 00:00:00');",
        )
        .await
        .unwrap();
    let due = read_due(&store, IdentityPhase::Keys, moment(0), 500)
        .await
        .unwrap();
    assert_eq!(row(&due, "null").spend, Decimal::ZERO);
    assert_eq!(row(&due, "null").max_budget, None);
    assert_eq!(row(&due, "nan").spend, Decimal::ZERO);
    assert_eq!(row(&due, "nan").max_budget, None);
    assert_eq!(row(&due, "infinite").spend, Decimal::ZERO);
    assert_eq!(row(&due, "infinite").max_budget, None);
    assert_eq!(row(&due, "finite").spend, Decimal::new(125, 1));
    assert_eq!(row(&due, "finite").max_budget, Some(Decimal::new(105, 1)));
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn key_reads_fail_with_a_connect_fault_without_a_pool() {
    let store = PostgresStore { pool: None };
    let error = read_due(&store, IdentityPhase::Keys, moment(0), 10)
        .await
        .unwrap_err();
    assert_eq!(error.error_type, ErrorType::Connect);
}

#[tokio::test]
async fn key_reads_report_an_other_fault_when_the_table_is_missing() {
    let (fixture, store, client) = provision(Vec::new()).await;
    let error = read_due(&store, IdentityPhase::Keys, moment(0), 10)
        .await
        .unwrap_err();
    assert_eq!(error.error_type, ErrorType::Other);
    assert!(error.message.contains("database SQLSTATE"));
    finish(fixture, store, client).await;
}
