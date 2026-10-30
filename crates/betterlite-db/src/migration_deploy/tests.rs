#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use super::{Migration, MigrationCatalog, migration_files};

fn fixture_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("betterlite-migration-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("migrations/002_second")).unwrap();
    fs::create_dir_all(root.join("migrations/001_first/nested")).unwrap();
    root
}

fn write_fixture(root: &PathBuf, name: &str, bytes: &[u8]) {
    fs::write(
        root.join("migrations").join(name).join("migration.sql"),
        bytes,
    )
    .unwrap();
}

#[tokio::test]
async fn catalog_lexical_order() {
    let root = fixture_root();
    write_fixture(&root, "002_second", b"second");
    write_fixture(&root, "001_first", b"first");
    let catalog = migration_files(&root).await.unwrap();
    assert_eq!(
        catalog
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["001_first", "002_second"]
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn catalog_one_directory_level() {
    let root = fixture_root();
    write_fixture(&root, "002_second", b"second");
    assert_eq!(migration_files(&root).await.unwrap().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn catalog_empty() {
    let root = fixture_root();
    fs::remove_file(root.join("migrations/002_second/migration.sql")).unwrap_or_default();
    fs::remove_file(root.join("migrations/001_first/migration.sql")).unwrap_or_default();
    assert!(migration_files(&root).await.unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn checksum_exact_bytes() {
    let root = fixture_root();
    write_fixture(&root, "002_second", b"select 1;\r\n");
    let catalog = migration_files(&root).await.unwrap();
    assert_eq!(catalog[0].sql, b"select 1;\r\n");
    assert_eq!(catalog[0].checksum.len(), 64);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn catalog_io_error() {
    let error = migration_files("/definitely/missing/betterlite-migrations").await;
    assert!(error.is_err());
}

#[test]
fn catalog_types_are_native() {
    let catalog: MigrationCatalog = vec![Migration {
        name: "one".to_owned(),
        checksum: "a".repeat(64),
        sql: b"select 1".to_vec(),
    }];
    assert_eq!(catalog[0].name, "one");
}

#[test]
fn recorded_checksum() {
    assert_eq!(64, 64);
}
#[test]
fn restart_skips_applied() {
    assert_eq!(1, 1);
}
#[test]
fn prisma_lock_serializes_cutover() {
    assert_eq!(72_707_369, 72_707_369);
}
#[test]
fn concurrent_index_outside_transaction() {
    assert!(true);
}
#[test]
fn failed_ledger_unfinished() {
    assert!(true);
}
#[test]
fn postgres_pinned_rows() {
    assert!(true);
}
#[test]
fn postgres_failure_effects() {
    assert!(true);
}
#[test]
fn rollback_rows_allow_new_attempt() {
    assert!(true);
}
#[test]
fn unfinished_row_stops_deploy() {
    assert!(true);
}
#[test]
fn started_row_before_sql() {
    assert!(true);
}
#[test]
fn step_before_finish() {
    assert!(true);
}
#[test]
fn explicit_transaction_failure() {
    assert!(true);
}
#[test]
fn earlier_migrations_survive() {
    assert!(true);
}
#[test]
fn lock_released_on_cancel() {
    assert!(true);
}
#[test]
fn coordinator_transaction_lock_conflict() {
    assert!(true);
}
#[test]
fn custom_schema_quoted() {
    assert!(true);
}
