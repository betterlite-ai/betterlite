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

pub async fn writer_is_read_only(pool: &Pool) -> Result<bool, WriterProbeError> {
    let connection = pool
        .get()
        .await
        .map_err(|source| WriterProbeError::Pool { source })?;
    let rows = connection
        .query(WRITER_READ_ONLY_PROBE_SQL, &[])
        .await
        .map_err(|source| WriterProbeError::Query { source })?;
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
    let connection = pool
        .get()
        .await
        .map_err(|source| WriterProbeError::Pool { source })?;
    connection
        .query(WRITER_SELECT_ONE_SQL, &[])
        .await
        .map_err(|source| WriterProbeError::Query { source })?;
    Ok(())
}
