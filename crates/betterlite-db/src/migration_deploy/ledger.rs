use std::collections::HashSet;
use std::str;

use tokio_postgres::{Client, Error as PostgresError};
use uuid::Uuid;

use super::DeployError;
use super::catalog::Migration;

pub(super) async fn prepare(client: &Client, table: &str) -> Result<HashSet<String>, DeployError> {
    client.batch_execute(&format!("CREATE TABLE IF NOT EXISTS {table} (id varchar(36) PRIMARY KEY NOT NULL, checksum varchar(64) NOT NULL, finished_at timestamptz, migration_name varchar(255) NOT NULL, logs text, rolled_back_at timestamptz, started_at timestamptz NOT NULL DEFAULT now(), applied_steps_count integer NOT NULL DEFAULT 0)")).await?;
    let rows = client.query(&format!("SELECT migration_name, finished_at IS NOT NULL, rolled_back_at IS NOT NULL FROM {table}"), &[]).await?;
    let mut finished = HashSet::new();
    for row in rows {
        let name: String = row.try_get(0)?;
        if !row.try_get::<_, bool>(2)? {
            if !row.try_get::<_, bool>(1)? {
                return Err(DeployError::Unfinished { name });
            }
            finished.insert(name);
        }
    }
    Ok(finished)
}

pub(super) async fn apply(
    client: &Client,
    table: &str,
    migration: &Migration,
    identifier: Uuid,
) -> Result<(), DeployError> {
    let id = identifier.to_string();
    client.execute(&format!("INSERT INTO {table} (id, checksum, migration_name, started_at, finished_at, rolled_back_at, logs, applied_steps_count) VALUES ($1, $2, $3, CURRENT_TIMESTAMP, NULL, NULL, NULL, 0)"), &[&id, &migration.checksum, &migration.name]).await?;
    execute(client, table, migration, &id).await?;
    client
        .execute(
            &format!("UPDATE {table} SET applied_steps_count = 1 WHERE id = $1"),
            &[&id],
        )
        .await?;
    client
        .execute(
            &format!("UPDATE {table} SET finished_at = CURRENT_TIMESTAMP WHERE id = $1"),
            &[&id],
        )
        .await?;
    Ok(())
}

async fn execute(
    client: &Client,
    table: &str,
    migration: &Migration,
    id: &str,
) -> Result<(), DeployError> {
    let sql = decode_sql(client, table, migration, id).await?;
    // The simple query protocol preserves explicit transactions and single-statement concurrent DDL.
    if let Err(source) = client.batch_execute(sql).await {
        let message = source
            .as_db_error()
            .map_or_else(|| source.to_string(), ToString::to_string);
        let name = migration.name.clone();
        return Err(match record_failure(client, table, id, &message).await {
            Ok(()) => DeployError::Migration { name, source },
            Err(ledger) => DeployError::MigrationLog {
                name,
                source,
                ledger,
            },
        });
    }
    Ok(())
}

async fn decode_sql<'migration>(
    client: &Client,
    table: &str,
    migration: &'migration Migration,
    id: &str,
) -> Result<&'migration str, DeployError> {
    match str::from_utf8(&migration.sql) {
        Ok(sql) => Ok(sql),
        Err(source) => {
            let name = migration.name.clone();
            Err(
                match record_failure(client, table, id, &source.to_string()).await {
                    Ok(()) => DeployError::InvalidSql { name, source },
                    Err(ledger) => DeployError::InvalidSqlLog {
                        name,
                        source,
                        ledger,
                    },
                },
            )
        }
    }
}

async fn record_failure(
    client: &Client,
    table: &str,
    id: &str,
    message: &str,
) -> Result<(), PostgresError> {
    client.batch_execute("ROLLBACK").await?;
    client
        .execute(
            &format!("UPDATE {table} SET logs = $2 WHERE id = $1"),
            &[&id, &message],
        )
        .await?;
    Ok(())
}
