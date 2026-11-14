#![cfg(test)]

use std::path::Path;
use std::str::from_utf8;
use std::sync::Mutex;

use rstest::rstest;
use tokio_postgres::Config;

use crate::migration_deploy::catalog::migration_files;

use super::{
    BEGIN_SQL, COMMIT_SQL, ExecutesSpendPartitionSteps, ROLLBACK_SQL, ROLLBACK_STEP,
    SpendPartitionError, SpendPartitionFuture, SpendPartitionOperation, SpendPartitionStep,
    run_conversion,
};

mod postgres;

const PARTITION_RENAMES: [(&str, &str); 6] = [
    ("LiteLLM_SpendLogs_pkey", "LiteLLM_SpendLogs_legacy_pkey"),
    (
        "LiteLLM_SpendLogs_startTime_idx",
        "LiteLLM_SpendLogs_legacy_startTime_idx",
    ),
    (
        "LiteLLM_SpendLogs_startTime_request_id_idx",
        "LiteLLM_SpendLogs_legacy_startTime_request_id_idx",
    ),
    (
        "LiteLLM_SpendLogs_end_user_idx",
        "LiteLLM_SpendLogs_legacy_end_user_idx",
    ),
    (
        "LiteLLM_SpendLogs_session_id_idx",
        "LiteLLM_SpendLogs_legacy_session_id_idx",
    ),
    (
        "LiteLLM_SpendLogs_api_key_startTime_idx",
        "LiteLLM_SpendLogs_legacy_api_key_startTime_idx",
    ),
];

const UNPARTITION_RENAMES: [(&str, &str); 7] = [
    (
        "LiteLLM_SpendLogs_pkey",
        "LiteLLM_SpendLogs_partitioned_pkey",
    ),
    (
        "LiteLLM_SpendLogs_pkey1",
        "LiteLLM_SpendLogs_partitioned_pkey1",
    ),
    (
        "LiteLLM_SpendLogs_startTime_idx",
        "LiteLLM_SpendLogs_partitioned_startTime_idx",
    ),
    (
        "LiteLLM_SpendLogs_startTime_request_id_idx",
        "LiteLLM_SpendLogs_partitioned_startTime_request_id_idx",
    ),
    (
        "LiteLLM_SpendLogs_end_user_idx",
        "LiteLLM_SpendLogs_partitioned_end_user_idx",
    ),
    (
        "LiteLLM_SpendLogs_session_id_idx",
        "LiteLLM_SpendLogs_partitioned_session_id_idx",
    ),
    (
        "LiteLLM_SpendLogs_api_key_startTime_idx",
        "LiteLLM_SpendLogs_partitioned_api_key_startTime_idx",
    ),
];

fn rename_sql(from: &str, to: &str) -> String {
    format!(r#"ALTER INDEX IF EXISTS "{from}" RENAME TO "{to}""#)
}

fn driver_error() -> tokio_postgres::Error {
    "unknown_option=on"
        .parse::<Config>()
        .expect_err("an unknown connection option")
}

#[derive(Clone, Copy, Default)]
enum Behavior {
    #[default]
    Pass,
    Fail(&'static str),
    FailBoth(&'static str),
}

#[derive(Default)]
struct Scripted {
    steps: Mutex<Vec<SpendPartitionStep>>,
    behavior: Behavior,
}

impl Scripted {
    fn failing_at(label: &'static str) -> Self {
        Self {
            steps: Mutex::new(Vec::new()),
            behavior: Behavior::Fail(label),
        }
    }

    fn rollback_failing(label: &'static str) -> Self {
        Self {
            steps: Mutex::new(Vec::new()),
            behavior: Behavior::FailBoth(label),
        }
    }

    fn recorded(&self) -> Vec<SpendPartitionStep> {
        self.steps.lock().expect("the step log").clone()
    }
}

impl ExecutesSpendPartitionSteps for Scripted {
    fn execute_spend_partition_step(&self, step: SpendPartitionStep) -> SpendPartitionFuture<'_> {
        self.steps.lock().expect("the step log").push(step);
        let failed = match self.behavior {
            Behavior::Pass => false,
            Behavior::Fail(label) => step.label == label,
            Behavior::FailBoth(label) => step.label == label || step.sql == ROLLBACK_SQL,
        };
        Box::pin(async move { if failed { Err(driver_error()) } else { Ok(()) } })
    }
}

#[test]
fn operation_names() {
    assert_eq!(
        SpendPartitionOperation::Partition.name(),
        "partition-spend-logs"
    );
    assert_eq!(
        SpendPartitionOperation::Unpartition.name(),
        "unpartition-spend-logs"
    );
}

#[tokio::test]
async fn partition_populated_table() {
    let scripted = Scripted::default();
    run_conversion(&scripted, SpendPartitionOperation::Partition)
        .await
        .expect("the partition conversion");
    let recorded = scripted.recorded();
    assert_eq!(
        recorded.as_slice(),
        SpendPartitionOperation::Partition.steps()
    );
    assert_eq!(recorded.len(), 17);
    assert_eq!(recorded.first().map(|step| step.sql), Some(BEGIN_SQL));
    assert_eq!(recorded.last().map(|step| step.sql), Some(COMMIT_SQL));
    let script = SpendPartitionOperation::Partition.script();
    assert!(script.starts_with("BEGIN;\n"));
    assert!(script.ends_with("COMMIT;\n"));
    assert!(!script.contains("INSERT"));
    assert!(!script.contains("SELECT"));
    assert!(!script.contains("DROP"));
}

#[test]
fn legacy_index_renames() {
    let legacy = SpendPartitionOperation::Partition.script();
    let moved = SpendPartitionOperation::Unpartition.script();
    let table_rename_at = legacy
        .find(r#"ALTER TABLE "LiteLLM_SpendLogs" RENAME TO "LiteLLM_SpendLogs_legacy""#)
        .expect("the legacy table rename");
    let create_at = legacy
        .find(r#"CREATE TABLE "LiteLLM_SpendLogs""#)
        .expect("the new canonical table");
    assert!(PARTITION_RENAMES.iter().all(|&(from, to)| {
        legacy
            .find(&rename_sql(from, to))
            .is_some_and(|at| at > table_rename_at && at < create_at)
    }));
    assert!(
        UNPARTITION_RENAMES
            .iter()
            .all(|&(from, to)| moved.contains(&rename_sql(from, to)))
    );
    assert!(!legacy.contains("pkey1"));
    assert!(moved.contains(r#""LiteLLM_SpendLogs_pkey1""#));
}

#[test]
fn composite_primary_key() {
    let partition = SpendPartitionOperation::Partition.script();
    let restore = SpendPartitionOperation::Unpartition.script();
    assert!(partition.contains(
        r#"ALTER TABLE "LiteLLM_SpendLogs" ADD PRIMARY KEY ("request_id", "startTime")"#
    ));
    assert!(restore.contains(r#"ALTER TABLE "LiteLLM_SpendLogs" ADD PRIMARY KEY ("request_id")"#));
    assert!(!restore.contains(r#""request_id", "startTime""#));
    assert!(
        partition
            .contains(r#"LIKE "LiteLLM_SpendLogs_legacy" INCLUDING DEFAULTS INCLUDING GENERATED"#)
    );
    assert!(restore.contains(
        r#"LIKE "LiteLLM_SpendLogs_partitioned" INCLUDING DEFAULTS INCLUDING GENERATED"#
    ));
    assert!(partition.contains(r#"PARTITION BY RANGE ("startTime")"#));
    assert!(!restore.contains("PARTITION BY"));
    let create_at = partition
        .find(r#"CREATE TABLE "LiteLLM_SpendLogs""#)
        .expect("the new canonical table");
    let primary_at = partition
        .find("ADD PRIMARY KEY")
        .expect("the composite primary key");
    let default_at = partition
        .find(r#"CREATE TABLE IF NOT EXISTS "LiteLLM_SpendLogs_pdefault""#)
        .expect("the default partition");
    assert!(create_at < primary_at);
    assert!(primary_at < default_at);
}

#[test]
fn unpartition_deduplication() {
    let script = SpendPartitionOperation::Unpartition.script();
    let copy = r#"INSERT INTO "LiteLLM_SpendLogs" SELECT * FROM "LiteLLM_SpendLogs_partitioned" ON CONFLICT ("request_id") DO NOTHING"#;
    assert!(script.contains(copy));
    assert!(!script.contains("ORDER BY"));
    assert!(!script.contains("DISTINCT"));
    assert!(!script.to_uppercase().contains("MAX("));
    assert!(!script.contains("created_at"));
    let api_key_at = script
        .find(r#"CREATE INDEX IF NOT EXISTS "LiteLLM_SpendLogs_api_key_startTime_idx""#)
        .expect("the last recreated index");
    let copy_at = script.find(copy).expect("the row copy");
    let drop_at = script
        .find(r#"DROP TABLE "LiteLLM_SpendLogs_partitioned""#)
        .expect("the partitioned table drop");
    let commit_at = script.find("COMMIT").expect("the commit");
    assert!(api_key_at < copy_at);
    assert!(copy_at < drop_at);
    assert!(drop_at < commit_at);
}

#[rstest]
#[case::partition_rename(
    SpendPartitionOperation::Partition,
    "renaming LiteLLM_SpendLogs to LiteLLM_SpendLogs_legacy"
)]
#[case::partition_index(
    SpendPartitionOperation::Partition,
    "creating LiteLLM_SpendLogs_session_id_idx"
)]
#[case::partition_default(SpendPartitionOperation::Partition, "creating the default partition")]
#[case::partition_commit(SpendPartitionOperation::Partition, "committing the transaction")]
#[case::unpartition_rename(
    SpendPartitionOperation::Unpartition,
    "renaming LiteLLM_SpendLogs to LiteLLM_SpendLogs_partitioned"
)]
#[case::unpartition_copy(
    SpendPartitionOperation::Unpartition,
    "copying rows into LiteLLM_SpendLogs"
)]
#[case::unpartition_drop(
    SpendPartitionOperation::Unpartition,
    "dropping LiteLLM_SpendLogs_partitioned"
)]
#[case::unpartition_commit(SpendPartitionOperation::Unpartition, "committing the transaction")]
#[tokio::test]
async fn failure_rolls_back(
    #[case] operation: SpendPartitionOperation,
    #[case] label: &'static str,
) {
    let scripted = Scripted::failing_at(label);
    let error = run_conversion(&scripted, operation)
        .await
        .expect_err("a failing step");
    assert!(matches!(error, SpendPartitionError::Statement { .. }));
    assert!(error.to_string().contains(label));
    assert!(error.to_string().contains(operation.name()));
    let recorded = scripted.recorded();
    let mut expected: Vec<SpendPartitionStep> = operation
        .steps()
        .iter()
        .copied()
        .take_while(|step| step.label != label)
        .collect();
    expected.push(
        *operation
            .steps()
            .iter()
            .find(|step| step.label == label)
            .expect("the failing step"),
    );
    expected.push(ROLLBACK_STEP);
    assert_eq!(recorded, expected);
    assert_eq!(
        recorded
            .iter()
            .filter(|step| step.sql == ROLLBACK_SQL)
            .count(),
        1
    );
    assert_eq!(recorded.last(), Some(&ROLLBACK_STEP));
}

#[tokio::test]
async fn rollback_failure_is_reported() {
    let scripted = Scripted::rollback_failing("creating the default partition");
    let error = run_conversion(&scripted, SpendPartitionOperation::Partition)
        .await
        .expect_err("a failing rollback");
    assert!(matches!(error, SpendPartitionError::Rollback { .. }));
    assert!(error.to_string().contains("the rollback also failed"));
    assert!(error.to_string().contains("creating the default partition"));
    assert_eq!(scripted.recorded().last(), Some(&ROLLBACK_STEP));
}

#[tokio::test]
async fn startup_no_conversion() {
    let catalog = migration_files(Path::new(env!("CARGO_MANIFEST_DIR")))
        .await
        .expect("the migration catalog");
    assert!(!catalog.is_empty());
    let statements: Vec<&str> = catalog
        .iter()
        .map(|migration| from_utf8(&migration.sql).expect("UTF-8 migration SQL"))
        .collect();
    let conversion: Vec<&str> = [
        SpendPartitionOperation::Partition,
        SpendPartitionOperation::Unpartition,
    ]
    .iter()
    .flat_map(|operation| operation.steps())
    .map(|step| step.sql)
    .filter(|sql| *sql != BEGIN_SQL && *sql != COMMIT_SQL)
    .collect();
    assert!(
        statements
            .iter()
            .all(|statement| conversion.iter().all(|sql| !statement.contains(*sql)))
    );
    assert!(
        statements
            .iter()
            .all(|statement| !statement.contains("PARTITION BY RANGE"))
    );
    assert!(
        catalog
            .iter()
            .all(|migration| !migration.name.contains("partition"))
    );
}

#[tokio::test]
async fn postgres_pinned_rows() {
    postgres::partition_retains_legacy_without_copy().await;
    postgres::defaults_generated_columns().await;
    postgres::canonical_index_definitions().await;
    postgres::missing_old_indexes().await;
    postgres::default_partition_accepts_rows().await;
    postgres::same_id_distinct_start_time().await;
    postgres::rollback_restores_single_key().await;
    postgres::alternate_pkey1_rename().await;
    postgres::duplicate_survivor_is_unspecified().await;
    postgres::drop_removes_children().await;
}

#[tokio::test]
async fn postgres_failure_effects() {
    postgres::destination_collision_rollback().await;
    postgres::mid_copy_failure_rollback().await;
}
