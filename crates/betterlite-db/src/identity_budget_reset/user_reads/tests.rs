#![cfg(test)]

use rust_decimal::Decimal;

use crate::database_faults::ErrorType;
use crate::identity_budget_reset::phase::IdentityPhase;
use crate::identity_budget_reset::rows::{DueIdentity, read_due};
use crate::identity_budget_reset::tests::support::{finish, identifiers, moment, schema};
use crate::postgres_store::session::PostgresStore;

fn row<'due>(due: &'due [DueIdentity], identifier: &str) -> &'due DueIdentity {
    due.iter().find(|row| row.identifier == identifier).unwrap()
}

#[tokio::test]
async fn user_reads_treat_a_null_reset_time_as_due() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_UserTable\" (user_id, spend, budget_duration, budget_reset_at) VALUES ('null-reset', 10, '1d', NULL), ('past', 10, '1d', '2025-12-30 00:00:00'), ('future', 10, '1d', '2026-06-01 00:00:00'), ('no-duration', 10, NULL, NULL);",
        )
        .await
        .unwrap();
    let due = read_due(&store, IdentityPhase::Users, moment(0), 500)
        .await
        .unwrap();
    assert_eq!(
        identifiers(&due),
        vec!["null-reset".to_owned(), "past".to_owned()]
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn user_reads_exclude_rows_without_a_budget_duration() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_UserTable\" (user_id, spend, budget_duration, budget_reset_at) VALUES ('no-duration', 10, NULL, '2025-12-30 00:00:00'), ('empty-duration', 10, '', '2025-12-30 00:00:00');",
        )
        .await
        .unwrap();
    let due = read_due(&store, IdentityPhase::Users, moment(0), 500)
        .await
        .unwrap();
    assert_eq!(identifiers(&due), vec!["empty-duration".to_owned()]);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn user_reads_stop_at_the_chunk_limit() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_UserTable\" (user_id, spend, budget_duration, budget_reset_at) VALUES ('a', 10, '1d', NULL), ('b', 10, '1d', NULL), ('c', 10, '1d', NULL);",
        )
        .await
        .unwrap();
    let due = read_due(&store, IdentityPhase::Users, moment(0), 2)
        .await
        .unwrap();
    assert_eq!(due.len(), 2);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn user_reads_convert_missing_and_non_finite_numbers_to_zero() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_UserTable\" (user_id, spend, max_budget, budget_duration, budget_reset_at) VALUES ('null', NULL, NULL, '1d', NULL), ('nan', 'NaN', 'NaN', '1d', NULL), ('finite', 7.5, 5.5, '1d', NULL);",
        )
        .await
        .unwrap();
    let due = read_due(&store, IdentityPhase::Users, moment(0), 500)
        .await
        .unwrap();
    assert_eq!(row(&due, "null").spend, Decimal::ZERO);
    assert_eq!(row(&due, "null").max_budget, None);
    assert_eq!(row(&due, "nan").spend, Decimal::ZERO);
    assert_eq!(row(&due, "nan").max_budget, None);
    assert_eq!(row(&due, "finite").spend, Decimal::new(75, 1));
    assert_eq!(row(&due, "finite").max_budget, Some(Decimal::new(55, 1)));
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn user_reads_fail_with_a_connect_fault_without_a_pool() {
    let store = PostgresStore { pool: None };
    let error = read_due(&store, IdentityPhase::Users, moment(0), 10)
        .await
        .unwrap_err();
    assert_eq!(error.error_type, ErrorType::Connect);
}
