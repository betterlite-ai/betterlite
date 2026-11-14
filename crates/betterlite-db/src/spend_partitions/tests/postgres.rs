use std::collections::BTreeMap;

use chrono::{NaiveDate, NaiveDateTime};
use secrecy::ExposeSecret as _;
use tokio::task::JoinSet;
use tokio_postgres::error::SqlState;
use tokio_postgres::{Client, NoTls, Row};

use crate::spend_partitions::fixtures::SpendPartitionCase;
use crate::spend_partitions::{SpendPartitionError, SpendPartitionOperation};

const RELKIND_SQL: &str =
    "SELECT relkind::text AS relkind FROM pg_class WHERE oid = to_regclass($1)";
const TABLE_NAME_SQL: &str = "SELECT to_regclass($1)::text AS name";
const CANONICAL_COUNT_SQL: &str = r#"SELECT count(*) AS rows FROM "LiteLLM_SpendLogs""#;
const LEGACY_COUNT_SQL: &str = r#"SELECT count(*) AS rows FROM "LiteLLM_SpendLogs_legacy""#;
const DEFAULT_COUNT_SQL: &str = r#"SELECT count(*) AS rows FROM "LiteLLM_SpendLogs_pdefault""#;
const INSERT_LOG_SQL: &str = r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, "startTime", "endTime") VALUES ($1, 'completion', 'sk-fixture', $2, $2)"#;
const COLUMNS_SQL: &str = r#"SELECT a.attname AS name,
       format_type(a.atttypid, a.atttypmod) AS column_type,
       a.attnotnull AS not_null,
       a.attgenerated::text AS generated,
       pg_get_expr(d.adbin, d.adrelid) AS default_expression
FROM pg_attribute a
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
WHERE a.attrelid = to_regclass($1) AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attnum"#;
const INDEXES_SQL: &str = r#"SELECT i.relname AS index_name,
       c.relname AS owner,
       ix.indisprimary AS is_primary,
       ix.indisunique AS is_unique,
       am.amname AS method,
       pg_get_indexdef(ix.indexrelid) AS definition
FROM pg_index ix
JOIN pg_class i ON i.oid = ix.indexrelid
JOIN pg_class c ON c.oid = ix.indrelid
JOIN pg_am am ON am.oid = i.relam
WHERE c.relname = $1 AND c.relnamespace = current_schema()::regnamespace
ORDER BY i.relname"#;
const PRIMARY_KEY_SQL: &str = r#"SELECT i.relname AS index_name, pg_get_indexdef(ix.indexrelid) AS definition
FROM pg_index ix
JOIN pg_class i ON i.oid = ix.indexrelid
JOIN pg_class c ON c.oid = ix.indrelid
WHERE c.relname = $1 AND c.relnamespace = current_schema()::regnamespace AND ix.indisprimary"#;
const PARTITIONS_SQL: &str = r#"SELECT c.relname AS partition_name, pg_get_expr(c.relpartbound, c.oid) AS bound
FROM pg_inherits inherits
JOIN pg_class c ON c.oid = inherits.inhrelid
WHERE inherits.inhparent = to_regclass($1)
ORDER BY c.relname"#;
const CHILD_INDEXES_SQL: &str = r#"SELECT parent.relname AS parent_index, child.relname AS child_index
FROM pg_inherits inherits
JOIN pg_class child ON child.oid = inherits.inhrelid
JOIN pg_class parent ON parent.oid = inherits.inhparent
JOIN pg_index ix ON ix.indexrelid = child.oid
WHERE ix.indrelid = to_regclass($1)
ORDER BY parent.relname"#;
const PARTITION_DEFINITION_SQL: &str = r#"SELECT partstrat::text AS strategy, pg_get_partkeydef(partrelid) AS key
FROM pg_partitioned_table
WHERE partrelid = to_regclass($1)"#;

#[derive(Debug, PartialEq, Eq)]
struct Column {
    column_type: String,
    not_null: bool,
    generated: String,
    default_expression: Option<String>,
}

#[derive(Debug)]
struct Index {
    name: String,
    owner: String,
    primary: bool,
    unique: bool,
    method: String,
    definition: String,
}

#[derive(Debug, PartialEq, Eq)]
struct Partition {
    name: String,
    bound: String,
}

fn at(month: u32, day: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, month, day)
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .expect("a test timestamp")
}

async fn catalog_rows(case: &SpendPartitionCase, sql: &str, table: &str) -> Vec<Row> {
    case.client
        .query(sql, &[&table])
        .await
        .expect("a catalog query")
}

fn column_of(row: &Row) -> (String, Column) {
    (
        row.get("name"),
        Column {
            column_type: row.get("column_type"),
            not_null: row.get("not_null"),
            generated: row.get("generated"),
            default_expression: row.get("default_expression"),
        },
    )
}

fn index_of(row: &Row) -> Index {
    let name = row.get("index_name");
    let owner = row.get("owner");
    let primary = row.get("is_primary");
    let unique = row.get("is_unique");
    let method = row.get("method");
    let definition = row.get("definition");
    Index {
        name,
        owner,
        primary,
        unique,
        method,
        definition,
    }
}

fn partition_of(row: &Row) -> Partition {
    Partition {
        name: row.get("partition_name"),
        bound: row.get("bound"),
    }
}

async fn relkind(case: &SpendPartitionCase, table: &str) -> String {
    let row = catalog_rows(case, RELKIND_SQL, table)
        .await
        .pop()
        .expect("the table relkind");
    row.get("relkind")
}

async fn table_name(case: &SpendPartitionCase, table: &str) -> Option<String> {
    let row = catalog_rows(case, TABLE_NAME_SQL, table).await;
    row.pop().and_then(|row| row.try_get("name").ok())
}

async fn row_count(case: &SpendPartitionCase, sql: &str) -> i64 {
    let row = case.client.query_one(sql, &[]).await.expect("a row count");
    row.get("rows")
}

async fn columns(case: &SpendPartitionCase, table: &str) -> BTreeMap<String, Column> {
    catalog_rows(case, COLUMNS_SQL, table)
        .await
        .iter()
        .map(column_of)
        .collect()
}

async fn indexes(case: &SpendPartitionCase, table: &str) -> Vec<Index> {
    catalog_rows(case, INDEXES_SQL, table)
        .await
        .iter()
        .map(index_of)
        .collect()
}

async fn primary_key(case: &SpendPartitionCase, table: &str) -> (String, String) {
    let row = catalog_rows(case, PRIMARY_KEY_SQL, table)
        .await
        .pop()
        .expect("the primary key");
    (row.get("index_name"), row.get("definition"))
}

async fn partitions(case: &SpendPartitionCase, parent: &str) -> Vec<Partition> {
    catalog_rows(case, PARTITIONS_SQL, parent)
        .await
        .iter()
        .map(partition_of)
        .collect()
}

async fn child_indexes(case: &SpendPartitionCase, partition: &str) -> Vec<String> {
    catalog_rows(case, CHILD_INDEXES_SQL, partition)
        .await
        .iter()
        .map(|row| row.get("parent_index"))
        .collect()
}

async fn partition_definition(case: &SpendPartitionCase, table: &str) -> Option<(String, String)> {
    catalog_rows(case, PARTITION_DEFINITION_SQL, table)
        .await
        .pop()
        .map(|row| (row.get("strategy"), row.get("key")))
}

fn assert_index(indexes: &[Index], name: &str, columns: &str, primary: bool) {
    let candidate = indexes
        .iter()
        .find(|entry| entry.name == name)
        .expect("the canonical index");
    assert_eq!(candidate.owner, "LiteLLM_SpendLogs");
    assert_eq!(candidate.primary, primary);
    assert_eq!(candidate.unique, primary);
    assert_eq!(candidate.method, "btree");
    assert!(
        candidate.definition.contains(columns),
        "{} does not define {}",
        candidate.definition,
        columns
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Canonical {
    Partitioned,
    Plain,
}

async fn assert_canonical(case: &SpendPartitionCase, kind: Canonical, rows: i64) {
    let expected = match kind {
        Canonical::Partitioned => "p",
        Canonical::Plain => "r",
    };
    assert_eq!(relkind(case, "LiteLLM_SpendLogs").await, expected);
    assert_eq!(row_count(case, CANONICAL_COUNT_SQL).await, rows);
    let indexes = indexes(case, "LiteLLM_SpendLogs").await;
    assert_eq!(indexes.len(), 6);
    assert!(
        indexes
            .iter()
            .all(|entry| entry.owner == "LiteLLM_SpendLogs")
    );
    let primary = primary_key(case, "LiteLLM_SpendLogs").await;
    assert_eq!(primary.0, "LiteLLM_SpendLogs_pkey");
    match kind {
        Canonical::Partitioned => {
            assert!(primary.1.contains(r#"(request_id, "startTime")"#));
            assert!(
                table_name(case, "LiteLLM_SpendLogs_partitioned")
                    .await
                    .is_none()
            );
        }
        Canonical::Plain => {
            assert!(primary.1.contains("(request_id)"));
            assert!(!primary.1.contains("startTime"));
        }
    }
}

pub(super) async fn partition_retains_legacy_without_copy() {
    let case = SpendPartitionCase::provision().await;
    case.seed_log("kept-one", at(1, 1)).await;
    case.seed_log("kept-two", at(2, 1)).await;
    case.run_ok(SpendPartitionOperation::Partition).await;
    assert_eq!(relkind(&case, "LiteLLM_SpendLogs_legacy").await, "r");
    assert_eq!(relkind(&case, "LiteLLM_SpendLogs").await, "p");
    assert_eq!(row_count(&case, LEGACY_COUNT_SQL).await, 2);
    assert_eq!(row_count(&case, CANONICAL_COUNT_SQL).await, 0);
    assert!(
        table_name(&case, "LiteLLM_SpendLogs_legacy")
            .await
            .is_some()
    );
    let definition = partition_definition(&case, "LiteLLM_SpendLogs")
        .await
        .expect("a partitioned table");
    assert_eq!(definition.0, "r");
    assert!(definition.1.contains(r#"RANGE ("startTime")"#));
    assert_eq!(
        partitions(&case, "LiteLLM_SpendLogs").await,
        vec![Partition {
            name: "LiteLLM_SpendLogs_pdefault".to_owned(),
            bound: "DEFAULT".to_owned(),
        }]
    );
    case.close().await;
}

pub(super) async fn defaults_generated_columns() {
    let case = SpendPartitionCase::provision().await;
    case.client
        .batch_execute(
            r#"ALTER TABLE "LiteLLM_SpendLogs" ADD COLUMN "fixture_generated_tokens" INTEGER GENERATED ALWAYS AS ("total_tokens" * 2) STORED"#,
        )
        .await
        .expect("a generated column");
    let before = columns(&case, "LiteLLM_SpendLogs").await;
    case.run_ok(SpendPartitionOperation::Partition).await;
    let after = columns(&case, "LiteLLM_SpendLogs").await;
    assert_eq!(before, after);
    assert_eq!(after.len(), 37);
    let start_time = after.get("startTime").expect("startTime");
    assert_eq!(start_time.column_type, "timestamp(3) without time zone");
    assert!(start_time.not_null);
    let spend = after.get("spend").expect("spend");
    assert_eq!(spend.column_type, "double precision");
    assert!(spend.default_expression.is_some());
    let metadata = after.get("metadata").expect("metadata");
    assert_eq!(metadata.column_type, "jsonb");
    assert!(
        metadata
            .default_expression
            .as_deref()
            .is_some_and(|expression| expression.contains("{}"))
    );
    let generated = after
        .get("fixture_generated_tokens")
        .expect("the generated column");
    assert_eq!(generated.generated, "s");
    assert!(
        generated
            .default_expression
            .as_deref()
            .is_some_and(|expression| expression.contains("total_tokens"))
    );
    case.close().await;
}

pub(super) async fn canonical_index_definitions() {
    let case = SpendPartitionCase::provision().await;
    case.run_ok(SpendPartitionOperation::Partition).await;
    let canonical = indexes(&case, "LiteLLM_SpendLogs").await;
    assert_eq!(canonical.len(), 6);
    assert!(
        canonical
            .iter()
            .all(|index| index.owner == "LiteLLM_SpendLogs")
    );
    assert_index(
        &canonical,
        "LiteLLM_SpendLogs_pkey",
        r#"(request_id, "startTime")"#,
        true,
    );
    assert_index(
        &canonical,
        "LiteLLM_SpendLogs_startTime_idx",
        r#"("startTime")"#,
        false,
    );
    assert_index(
        &canonical,
        "LiteLLM_SpendLogs_startTime_request_id_idx",
        r#"("startTime", request_id)"#,
        false,
    );
    assert_index(
        &canonical,
        "LiteLLM_SpendLogs_end_user_idx",
        "(end_user)",
        false,
    );
    assert_index(
        &canonical,
        "LiteLLM_SpendLogs_session_id_idx",
        "(session_id)",
        false,
    );
    assert_index(
        &canonical,
        "LiteLLM_SpendLogs_api_key_startTime_idx",
        r#"(api_key, "startTime")"#,
        false,
    );
    let mut propagated: Vec<String> = child_indexes(&case, "LiteLLM_SpendLogs_pdefault").await;
    propagated.sort();
    propagated.dedup();
    let mut expected: Vec<String> = [
        "LiteLLM_SpendLogs_api_key_startTime_idx",
        "LiteLLM_SpendLogs_end_user_idx",
        "LiteLLM_SpendLogs_pkey",
        "LiteLLM_SpendLogs_session_id_idx",
        "LiteLLM_SpendLogs_startTime_idx",
        "LiteLLM_SpendLogs_startTime_request_id_idx",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    expected.sort();
    assert_eq!(propagated, expected);
    let legacy = indexes(&case, "LiteLLM_SpendLogs_legacy").await;
    assert_eq!(legacy.len(), 6);
    assert!(
        legacy
            .iter()
            .all(|index| index.owner == "LiteLLM_SpendLogs_legacy")
    );
    assert!(
        legacy
            .iter()
            .all(|index| index.name.starts_with("LiteLLM_SpendLogs_legacy_"))
    );
    case.close().await;
}

pub(super) async fn missing_old_indexes() {
    let case = SpendPartitionCase::provision().await;
    case.client
        .batch_execute(
            r#"DROP INDEX "LiteLLM_SpendLogs_startTime_idx";
DROP INDEX "LiteLLM_SpendLogs_startTime_request_id_idx";
DROP INDEX "LiteLLM_SpendLogs_end_user_idx";
DROP INDEX "LiteLLM_SpendLogs_session_id_idx";
DROP INDEX "LiteLLM_SpendLogs_api_key_startTime_idx";"#,
        )
        .await
        .expect("the old indexes are dropped");
    case.run_ok(SpendPartitionOperation::Partition).await;
    let canonical = indexes(&case, "LiteLLM_SpendLogs").await;
    assert_eq!(canonical.len(), 6);
    assert!(
        canonical
            .iter()
            .any(|index| index.name == "LiteLLM_SpendLogs_startTime_idx")
    );
    assert!(
        canonical
            .iter()
            .any(|index| index.name == "LiteLLM_SpendLogs_api_key_startTime_idx")
    );
    let legacy = indexes(&case, "LiteLLM_SpendLogs_legacy").await;
    assert_eq!(legacy.len(), 1);
    assert_eq!(legacy[0].name, "LiteLLM_SpendLogs_legacy_pkey");
    case.close().await;
}

pub(super) async fn default_partition_accepts_rows() {
    let case = SpendPartitionCase::provision().await;
    case.run_ok(SpendPartitionOperation::Partition).await;
    case.seed_log("past", at(1, 1)).await;
    case.seed_log("future", at(12, 31)).await;
    let rows = case
        .client
        .query(
            r#"SELECT tableoid::regclass::text AS partition FROM "LiteLLM_SpendLogs" ORDER BY request_id"#,
            &[],
        )
        .await
        .expect("the routed rows");
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|row| row.get::<_, String>("partition") == "LiteLLM_SpendLogs_pdefault")
    );
    assert_eq!(
        partitions(&case, "LiteLLM_SpendLogs").await,
        vec![Partition {
            name: "LiteLLM_SpendLogs_pdefault".to_owned(),
            bound: "DEFAULT".to_owned(),
        }]
    );
    case.close().await;
}

pub(super) async fn same_id_distinct_start_time() {
    let case = SpendPartitionCase::provision().await;
    case.run_ok(SpendPartitionOperation::Partition).await;
    case.seed_log("shared", at(3, 1)).await;
    case.seed_log("shared", at(4, 1)).await;
    let error = case
        .client
        .execute(INSERT_LOG_SQL, &[&"shared", &at(3, 1)])
        .await
        .expect_err("a duplicate request_id at the same startTime");
    assert_eq!(error.code(), Some(&SqlState::UNIQUE_VIOLATION));
    assert_eq!(row_count(&case, CANONICAL_COUNT_SQL).await, 2);
    case.close().await;
}

pub(super) async fn rollback_restores_single_key() {
    let case = SpendPartitionCase::provision().await;
    case.seed_log("before", at(1, 1)).await;
    case.run_ok(SpendPartitionOperation::Partition).await;
    case.seed_log("after", at(2, 1)).await;
    case.run_ok(SpendPartitionOperation::Unpartition).await;
    assert_canonical(&case, Canonical::Plain, 1).await;
    assert!(
        partition_definition(&case, "LiteLLM_SpendLogs")
            .await
            .is_none()
    );
    assert!(partitions(&case, "LiteLLM_SpendLogs").await.is_empty());
    assert!(
        table_name(&case, "LiteLLM_SpendLogs_partitioned")
            .await
            .is_none()
    );
    assert!(
        table_name(&case, "LiteLLM_SpendLogs_legacy")
            .await
            .is_some()
    );
    assert_eq!(row_count(&case, LEGACY_COUNT_SQL).await, 1);
    let rows = case
        .client
        .query(
            r#"SELECT request_id FROM "LiteLLM_SpendLogs" ORDER BY request_id"#,
            &[],
        )
        .await
        .expect("the restored rows");
    let ids: Vec<String> = rows.iter().map(|row| row.get("request_id")).collect();
    assert_eq!(ids, vec!["after".to_owned()]);
    case.close().await;
}

pub(super) async fn alternate_pkey1_rename() {
    let case = SpendPartitionCase::provision().await;
    case.run_ok(SpendPartitionOperation::Partition).await;
    case.client
        .batch_execute(
            r#"ALTER INDEX "LiteLLM_SpendLogs_pkey" RENAME TO "LiteLLM_SpendLogs_pkey1""#,
        )
        .await
        .expect("the alternate primary key name");
    case.seed_log("survivor", at(5, 1)).await;
    case.run_ok(SpendPartitionOperation::Unpartition).await;
    let primary = primary_key(&case, "LiteLLM_SpendLogs").await;
    assert_eq!(primary.0, "LiteLLM_SpendLogs_pkey");
    assert!(primary.1.contains("(request_id)"));
    assert!(table_name(&case, "LiteLLM_SpendLogs_pkey1").await.is_none());
    assert_eq!(row_count(&case, CANONICAL_COUNT_SQL).await, 1);
    case.close().await;
}

pub(super) async fn duplicate_survivor_is_unspecified() {
    let case = SpendPartitionCase::provision().await;
    case.run_ok(SpendPartitionOperation::Partition).await;
    case.seed_log("duplicate", at(1, 1)).await;
    case.seed_log("duplicate", at(2, 1)).await;
    case.seed_log("duplicate", at(3, 1)).await;
    case.seed_log("distinct", at(1, 1)).await;
    case.run_ok(SpendPartitionOperation::Unpartition).await;
    let rows = case
        .client
        .query(
            r#"SELECT "startTime" FROM "LiteLLM_SpendLogs" WHERE request_id = 'duplicate'"#,
            &[],
        )
        .await
        .expect("the survivor");
    assert_eq!(rows.len(), 1);
    let start_time: NaiveDateTime = rows[0].get("startTime");
    assert!([at(1, 1), at(2, 1), at(3, 1)].contains(&start_time));
    assert_eq!(row_count(&case, CANONICAL_COUNT_SQL).await, 2);
    let distinct = case
        .client
        .query_one(
            r#"SELECT count(DISTINCT request_id) AS rows FROM "LiteLLM_SpendLogs""#,
            &[],
        )
        .await
        .expect("the distinct request ids");
    assert_eq!(distinct.get::<_, i64>("rows"), 2);
    case.close().await;
}

pub(super) async fn drop_removes_children() {
    let case = SpendPartitionCase::provision().await;
    case.run_ok(SpendPartitionOperation::Partition).await;
    case.client
        .batch_execute(
            r#"CREATE TABLE "LiteLLM_SpendLogs_20260101" PARTITION OF "LiteLLM_SpendLogs" FOR VALUES FROM ('2026-01-01') TO ('2026-01-02')"#,
        )
        .await
        .expect("a day partition");
    case.seed_log("day-row", at(1, 1)).await;
    case.seed_log("default-row", at(2, 1)).await;
    assert_eq!(partitions(&case, "LiteLLM_SpendLogs").await.len(), 2);
    case.run_ok(SpendPartitionOperation::Unpartition).await;
    assert_eq!(relkind(&case, "LiteLLM_SpendLogs").await, "r");
    assert_eq!(row_count(&case, CANONICAL_COUNT_SQL).await, 2);
    assert!(partitions(&case, "LiteLLM_SpendLogs").await.is_empty());
    assert!(
        table_name(&case, "LiteLLM_SpendLogs_20260101")
            .await
            .is_none()
    );
    assert!(
        table_name(&case, "LiteLLM_SpendLogs_pdefault")
            .await
            .is_none()
    );
    assert!(
        table_name(&case, "LiteLLM_SpendLogs_partitioned")
            .await
            .is_none()
    );
    case.close().await;
}

pub(super) async fn destination_collision_rollback() {
    let case = SpendPartitionCase::provision().await;
    case.seed_log("kept", at(1, 1)).await;
    case.client
        .batch_execute(r#"CREATE TABLE "LiteLLM_SpendLogs_legacy" (marker INTEGER)"#)
        .await
        .expect("the collision table");
    let error = case.run_error(SpendPartitionOperation::Partition).await;
    assert!(matches!(error, SpendPartitionError::Statement { .. }));
    assert!(error.to_string().contains("LiteLLM_SpendLogs_legacy"));
    assert_canonical(&case, Canonical::Plain, 1).await;
    assert!(
        table_name(&case, "LiteLLM_SpendLogs_pdefault")
            .await
            .is_none()
    );
    case.client
        .batch_execute(r#"DROP TABLE "LiteLLM_SpendLogs_legacy""#)
        .await
        .expect("the collision is removed");
    case.run_ok(SpendPartitionOperation::Partition).await;
    case.seed_log("live", at(2, 1)).await;
    case.client
        .batch_execute(r#"CREATE TABLE "LiteLLM_SpendLogs_partitioned" (marker INTEGER)"#)
        .await
        .expect("the collision table");
    let error = case.run_error(SpendPartitionOperation::Unpartition).await;
    assert!(matches!(error, SpendPartitionError::Statement { .. }));
    assert!(error.to_string().contains("LiteLLM_SpendLogs_partitioned"));
    assert_eq!(relkind(&case, "LiteLLM_SpendLogs").await, "p");
    assert_eq!(row_count(&case, CANONICAL_COUNT_SQL).await, 1);
    let primary = primary_key(&case, "LiteLLM_SpendLogs").await;
    assert_eq!(primary.0, "LiteLLM_SpendLogs_pkey");
    assert!(primary.1.contains(r#"(request_id, "startTime")"#));
    assert!(
        table_name(&case, "LiteLLM_SpendLogs_partitioned")
            .await
            .is_some()
    );
    case.close().await;
}

async fn lock_default_partition(
    case: &SpendPartitionCase,
) -> (Client, JoinSet<Result<(), tokio_postgres::Error>>) {
    let (client, connection) =
        tokio_postgres::connect(case.fixture.options.url.expose_secret(), NoTls)
            .await
            .expect("a lock connection");
    let mut tasks = JoinSet::new();
    tasks.spawn(connection);
    client
        .batch_execute(r#"BEGIN; LOCK TABLE "LiteLLM_SpendLogs_pdefault" IN ACCESS EXCLUSIVE MODE"#)
        .await
        .expect("the default partition lock");
    (client, tasks)
}

pub(super) async fn mid_copy_failure_rollback() {
    let case = SpendPartitionCase::provision().await;
    case.run_ok(SpendPartitionOperation::Partition).await;
    case.seed_log("first", at(1, 1)).await;
    case.seed_log("second", at(2, 1)).await;
    let (lock_client, lock_tasks) = lock_default_partition(&case).await;
    case.client
        .batch_execute("SET statement_timeout = '2s'")
        .await
        .expect("a statement timeout");
    let error = case.run_error(SpendPartitionOperation::Unpartition).await;
    assert!(matches!(error, SpendPartitionError::Statement { .. }));
    assert!(
        error
            .to_string()
            .contains("copying rows into LiteLLM_SpendLogs")
    );
    lock_client
        .batch_execute("COMMIT")
        .await
        .expect("the lock release");
    drop(lock_client);
    drop(lock_tasks);
    assert_canonical(&case, Canonical::Partitioned, 2).await;
    assert_eq!(row_count(&case, DEFAULT_COUNT_SQL).await, 2);
    case.close().await;
}
