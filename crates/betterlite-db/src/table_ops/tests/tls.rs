#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use super::support::{Harness, Ports, TableClient, wait_for_idle};
use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenColumn;
use crate::table_ops::generic::{GenericReads as _, KeyFilter};
use crate::table_ops::updates::batches::BatchUpdates as _;
use crate::table_ops::{
    DEFAULT_ACQUIRE_TIMEOUT, DEFAULT_TEAM_LIST_LIMIT, TableOpsError, TableSettings,
};

mod front;

#[rstest]
#[case::read(false)]
#[case::batch(true)]
#[tokio::test]
async fn tls_timeout_stops_server_work_while_the_lock_stays_held(#[case] batch: bool) {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.unwrap();
    db.batch_execute(
        r#"INSERT INTO "LiteLLM_VerificationToken" (token, key_alias, models)
        VALUES ('tls-token', 'original', ARRAY[]::text[])"#,
    )
    .await
    .unwrap();
    db.batch_execute(r#"BEGIN; LOCK TABLE "LiteLLM_VerificationToken" IN ACCESS EXCLUSIVE MODE"#)
        .await
        .unwrap();
    let front = front::Front::start(&harness.fixture.options.url).await;
    let client = TableClient::new(front.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations_with(
        &client,
        TableSettings {
            team_list_limit: DEFAULT_TEAM_LIST_LIMIT,
            acquire_timeout: DEFAULT_ACQUIRE_TIMEOUT,
            query_timeout: Duration::from_millis(100),
        },
    );
    let error = if batch {
        ops.update_many_keys(&[BTreeMap::from([
            (
                TokenColumn::Token,
                TokenValue::Text(String::from("tls-token")),
            ),
            (
                TokenColumn::KeyAlias,
                TokenValue::Text(String::from("changed")),
            ),
        ])])
        .await
        .unwrap_err()
    } else {
        ops.generic_row(KeyFilter::KeyAlias("original"))
            .await
            .unwrap_err()
    };
    let expected_operation = if batch { "statement" } else { "query" };
    assert!(
        matches!(error, TableOpsError::Timeout { operation, .. } if operation == expected_operation)
    );
    assert_eq!(front.pool.status().size, 0);
    let observer = harness.pool.get().await.unwrap();
    wait_for_idle(&observer, "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND application_name = 'table-ops-tls' AND state = 'active'").await;
    db.batch_execute("ROLLBACK").await.unwrap();
    let alias: String = observer
        .query_one(
            r#"SELECT key_alias FROM "LiteLLM_VerificationToken" WHERE token = 'tls-token'"#,
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(alias, "original");
    drop(front);
    harness.close().await;
}

#[tokio::test]
async fn statement_deadline_does_not_leak_to_next_borrower() {
    let harness = Harness::start().await;
    let pool = harness.pool_with_size(1);
    let db = pool.get().await.unwrap();
    db.batch_execute("SET statement_timeout = '7654ms'")
        .await
        .unwrap();
    drop(db);
    let client = TableClient::new(pool.clone());
    let ports = Ports::new();
    assert!(
        ports
            .operations(&client)
            .generic_row(KeyFilter::KeyAlias("absent"))
            .await
            .unwrap()
            .is_none()
    );
    let borrowed = pool.get().await.unwrap();
    let value: String = borrowed
        .query_one("SHOW statement_timeout", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(value, "7654ms");
    harness.close().await;
}
