use deadpool_postgres::Pool;

pub const WRITER_READ_ONLY_PROBE_SQL: &str =
    "SELECT current_setting('transaction_read_only') AS transaction_read_only";
pub const WRITER_SELECT_ONE_SQL: &str = "SELECT 1";

#[derive(Debug, thiserror::Error)]
pub enum WriterProbeError {
    #[error("the writer pool has no available connection")]
    Pool {
        #[source]
        source: deadpool_postgres::PoolError,
    },
    #[error("the writer probe query failed")]
    Query {
        #[source]
        source: tokio_postgres::Error,
    },
}

async fn probe_rows(pool: &Pool, sql: &str) -> Result<Vec<tokio_postgres::Row>, WriterProbeError> {
    let connection = pool
        .get()
        .await
        .map_err(|source| WriterProbeError::Pool { source })?;
    connection
        .query(sql, &[])
        .await
        .map_err(|source| WriterProbeError::Query { source })
}

pub async fn writer_is_read_only(pool: &Pool) -> Result<bool, WriterProbeError> {
    let rows = probe_rows(pool, WRITER_READ_ONLY_PROBE_SQL).await?;
    for row in &rows {
        let value = row
            .try_get::<_, String>("transaction_read_only")
            .map_err(|source| WriterProbeError::Query { source })?;
        if value == "on" {
            return Ok(true);
        }
    }
    Ok(false)
}

pub async fn writer_select_one(pool: &Pool) -> Result<(), WriterProbeError> {
    probe_rows(pool, WRITER_SELECT_ONE_SQL).await?;
    Ok(())
}
