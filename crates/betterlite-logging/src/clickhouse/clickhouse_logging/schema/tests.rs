#![cfg(test)]

use super::{SPEND_LOGS_TABLE, SchemaError, ensure_schema};
use crate::clickhouse::clickhouse_logging::tests::support::FakeStorage;

#[test]
fn the_spend_logs_table_name_is_litellms() {
    assert_eq!(SPEND_LOGS_TABLE, "spend_logs");
}

#[tokio::test]
async fn ensure_schema_forwards_the_retention_days() {
    let storage = FakeStorage::default();
    ensure_schema(&storage, 30, 90)
        .await
        .expect("the schema is ensured");
    assert_eq!(storage.schema_calls(), vec![(30, 90)]);
}

#[tokio::test]
async fn ensure_schema_keeps_the_storage_error() {
    let storage = FakeStorage::default();
    storage.fail_schema();
    let error = ensure_schema(&storage, 30, 90)
        .await
        .expect_err("the storage rejects the schema");
    assert!(matches!(error, SchemaError::Storage { .. }));
}
