#![cfg(test)]

use std::convert::Infallible;
use std::time::Duration;

use crate::table_ops::TableOpsError;
use crate::table_ops::tests::support::Harness;

#[tokio::test]
async fn server_timeout_is_reported_before_the_client_deadline() {
    let harness = Harness::start().await;
    let client = harness.pool.get().await.unwrap();
    client
        .batch_execute("SET statement_timeout = '1ms'")
        .await
        .unwrap();
    let error = super::statement::<Infallible, _, _>(
        "query",
        Duration::from_secs(2),
        client.query("SELECT pg_sleep(1)", &[]),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, TableOpsError::Timeout { operation: "query", deadline } if deadline == Duration::from_secs(2))
    );
    client
        .batch_execute("SET statement_timeout = 0")
        .await
        .unwrap();
    let ordinary = super::statement::<Infallible, _, _>(
        "query",
        Duration::from_secs(2),
        client.query("SELECT unknown_column", &[]),
    )
    .await
    .unwrap_err();
    assert!(matches!(ordinary, TableOpsError::Database { .. }));
    harness.close().await;
}
