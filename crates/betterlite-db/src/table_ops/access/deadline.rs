mod tests;

use std::error::Error;
use std::future::Future;
use std::time::Duration;

use deadpool_postgres::{Client, Transaction};
use tokio::time::timeout;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::ToSql;
use tokio_postgres::{Error as PgError, Row};

use crate::postgres_store::diagnostics::DriverError;
use crate::table_ops::TableOpsError;

pub(super) async fn statement<ClientError, Call, Output>(
    operation: &'static str,
    deadline: Duration,
    call: Call,
) -> Result<Output, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
    Call: Future<Output = Result<Output, PgError>>,
{
    match timeout(deadline, call).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(driver)) if driver.code() != Some(&SqlState::QUERY_CANCELED) => {
            Err(TableOpsError::from(DriverError { driver }))
        }
        Ok(Err(_)) | Err(_) => Err(TableOpsError::Timeout {
            operation,
            deadline,
        }),
    }
}

// Transaction-local deadlines bound server execution without changing the next borrower's settings.
pub(super) async fn begin<ClientError>(
    client: &mut Client,
    deadline: Duration,
) -> Result<Transaction<'_>, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
{
    let transaction = statement("begin", deadline, client.transaction()).await?;
    // PostgreSQL treats zero as unlimited, so sub-millisecond deadlines round up.
    let milliseconds = deadline.as_millis().clamp(1, u128::from(i32::MAX));
    let setting = format!("{milliseconds}ms");
    statement(
        "deadline",
        deadline,
        transaction.query_one(
            "SELECT set_config('statement_timeout', $1, true)",
            &[&setting],
        ),
    )
    .await?;
    Ok(transaction)
}

pub(super) async fn query<ClientError>(
    client: &mut Client,
    sql: &str,
    parameters: &[&(dyn ToSql + Sync)],
    deadline: Duration,
) -> Result<Vec<Row>, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
{
    let transaction = begin(client, deadline).await?;
    let rows = statement("query", deadline, transaction.query(sql, parameters)).await?;
    statement("commit", deadline, transaction.commit()).await?;
    Ok(rows)
}
