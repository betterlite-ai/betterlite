use std::future::Future;
use std::time::Duration;

use tokio::time::timeout;
use tokio_postgres::{Client, Error as PostgresError, Transaction};

use super::calendar::{PlannedPartition, SPEND_LOGS_TABLE};
use super::partition_bound::{CatalogPartition, decode_partition_bound};

pub type RemainingBudget<'budget> = &'budget mut (dyn FnMut() -> Option<u64> + Send);

pub const TRANSACTION_COMMIT_SLACK: Duration = Duration::from_secs(5);

const PARTITIONED_COLUMN: &str = "partitioned";
const NAME_COLUMN: &str = "name";
const BOUND_COLUMN: &str = "bound";

const PARTITIONED_SQL: &str = "SELECT EXISTS (
    SELECT 1
    FROM pg_partitioned_table pt
    JOIN pg_class c ON c.oid = pt.partrelid
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relname = $1
      AND n.nspname = current_schema()
) AS partitioned";

const PARTITIONS_SQL: &str = "SELECT c.relname AS name,
       pg_get_expr(c.relpartbound, c.oid) AS bound
FROM pg_inherits i
JOIN pg_class c ON c.oid = i.inhrelid
JOIN pg_class p ON p.oid = i.inhparent
JOIN pg_namespace n ON n.oid = p.relnamespace
WHERE p.relname = $1
  AND n.nspname = current_schema()";

#[derive(Debug, thiserror::Error)]
pub enum BoundedTransactionError {
    #[error(transparent)]
    Database(#[from] PostgresError),
    #[error("the spend-log partition statement did not finish within {millis} ms")]
    Deadline { millis: u64 },
}

// _bounded_tx: prisma's transaction timeout is the caller's statement budget plus room for the commit
pub(crate) async fn within_deadline<T, Work>(
    timeout_ms: u64,
    work: Work,
) -> Result<T, BoundedTransactionError>
where
    Work: Future<Output = Result<T, PostgresError>>,
{
    let deadline = Duration::from_millis(timeout_ms).saturating_add(TRANSACTION_COMMIT_SLACK);
    match timeout(deadline, work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(source)) => Err(BoundedTransactionError::Database(source)),
        Err(_elapsed) => Err(BoundedTransactionError::Deadline { millis: timeout_ms }),
    }
}

// partition DDL takes ACCESS EXCLUSIVE, so its lock wait is bounded alongside the statement
async fn bounded_transaction<'client>(
    client: &'client mut Client,
    timeout_ms: u64,
    with_lock_timeout: bool,
) -> Result<Transaction<'client>, PostgresError> {
    let transaction = client.transaction().await?;
    let settings = if with_lock_timeout {
        format!("SET LOCAL statement_timeout = {timeout_ms}; SET LOCAL lock_timeout = {timeout_ms}")
    } else {
        format!("SET LOCAL statement_timeout = {timeout_ms}")
    };
    transaction.batch_execute(&settings).await?;
    Ok(transaction)
}

async fn probe_partitioned(client: &mut Client, timeout_ms: u64) -> Result<bool, PostgresError> {
    let transaction = bounded_transaction(client, timeout_ms, false).await?;
    let row = transaction
        .query_one(PARTITIONED_SQL, &[&SPEND_LOGS_TABLE])
        .await?;
    let partitioned = row.try_get::<_, bool>(PARTITIONED_COLUMN)?;
    transaction.commit().await?;
    Ok(partitioned)
}

// is_partitioned: a catalog failure only means this run leaves the table alone
pub async fn is_partitioned(client: &mut Client, remaining: RemainingBudget<'_>) -> bool {
    let Some(timeout_ms) = remaining() else {
        return false;
    };
    match within_deadline(timeout_ms, probe_partitioned(client, timeout_ms)).await {
        Ok(partitioned) => partitioned,
        Err(error) => {
            tracing::warn!(
                "Could not determine if {SPEND_LOGS_TABLE} is partitioned, assuming it is not: {error}"
            );
            false
        }
    }
}

// list_partitions: unlike the probe, a failure here ends the cleanup attempt
pub async fn list_partitions(
    client: &mut Client,
    timeout_ms: u64,
) -> Result<Vec<CatalogPartition>, PostgresError> {
    let transaction = bounded_transaction(client, timeout_ms, false).await?;
    let rows = transaction
        .query(PARTITIONS_SQL, &[&SPEND_LOGS_TABLE])
        .await?;
    let mut partitions = Vec::with_capacity(rows.len());
    for row in rows {
        let name = row.try_get::<_, String>(NAME_COLUMN)?;
        let bound = row.try_get::<_, Option<String>>(BOUND_COLUMN)?;
        let expression = bound.as_deref().unwrap_or_default();
        partitions.push(CatalogPartition {
            name,
            bound: decode_partition_bound(expression.as_bytes()),
        });
    }
    transaction.commit().await?;
    Ok(partitions)
}

pub(crate) async fn execute_ddl(
    client: &mut Client,
    statement: &str,
    timeout_ms: u64,
) -> Result<(), PostgresError> {
    let transaction = bounded_transaction(client, timeout_ms, true).await?;
    transaction.batch_execute(statement).await?;
    transaction.commit().await
}

#[must_use]
pub fn create_partition_statement(partition: &PlannedPartition) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS \"{}\" PARTITION OF \"{}\" FOR VALUES FROM ('{}') TO ('{}')",
        partition.name.replace('"', "\"\""),
        SPEND_LOGS_TABLE.replace('"', "\"\""),
        partition.lower,
        partition.upper,
    )
}

#[must_use]
pub fn drop_partition_statement(catalog_partition_name: &str) -> String {
    format!(
        "DROP TABLE IF EXISTS \"{}\"",
        catalog_partition_name.replace('"', "\"\""),
    )
}
