use crate::schema_baseline::{BASELINE_MIGRATION, MIGRATION_CATALOG};
use sha2::{Digest as _, Sha256};

const PINNED_SHA256: &str = "0bf3d5cabe50d5d7249eb505cf2ec6fe2d8b49dc8b496dceb2179a357ed4725e";

#[test]
fn baseline_sql_identity() {
    let mut hasher = Sha256::new();
    hasher.update(BASELINE_MIGRATION.bytes);
    assert_eq!(hex::encode(hasher.finalize()), PINNED_SHA256);
    assert_eq!(BASELINE_MIGRATION.sha256, PINNED_SHA256);
    assert_eq!(BASELINE_MIGRATION.directory, "20250326162113_baseline");
    assert_eq!(BASELINE_MIGRATION.bytes.len(), 15_827);
    let sql = migration_sql();
    assert_eq!(sql.matches("CREATE TABLE IF NOT EXISTS").count(), 17);
    let regular_indexes = sql.matches("CREATE INDEX IF NOT EXISTS").count();
    let unique_indexes = sql.matches("CREATE UNIQUE INDEX IF NOT EXISTS").count();
    assert_eq!(regular_indexes + unique_indexes, 6);
    assert_eq!(sql.matches("ADD CONSTRAINT").count(), 14);
    assert!(sql.ends_with('\n'));
}

#[test]
fn original_directory_registration() {
    assert_eq!(MIGRATION_CATALOG.files, &[BASELINE_MIGRATION]);
}

#[test]
fn statement_order_tables_indexes_foreign_keys() {
    let sql = migration_sql();
    let (before_indexes, after_indexes) = sql.split_once("-- CreateIndex").unwrap();
    let (indexes, _) = after_indexes.split_once("-- AddForeignKey").unwrap();
    assert!(before_indexes.contains("-- CreateTable"));
    assert!(!indexes.is_empty());
}

#[test]
fn seventeen_tables_six_indexes_fourteen_foreign_keys() {
    let sql = migration_sql();
    assert_eq!(sql.matches("-- CreateTable").count(), 17);
    assert_eq!(sql.matches("-- CreateIndex").count(), 6);
    assert_eq!(sql.matches("-- AddForeignKey").count(), 14);
}
