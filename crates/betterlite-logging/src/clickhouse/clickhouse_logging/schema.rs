use super::storage::ClickHouseStorage;

pub const SPEND_LOGS_TABLE: &str = "spend_logs";

#[derive(Debug, thiserror::Error)]
pub enum SchemaError<StorageError> {
    #[error("the ClickHouse schema could not be ensured")]
    Storage {
        #[source]
        source: StorageError,
    },
}

pub async fn ensure_schema<Storage>(
    storage: &Storage,
    trace_retention_days: u32,
    spend_log_retention_days: u32,
) -> Result<(), SchemaError<Storage::Error>>
where
    Storage: ClickHouseStorage,
{
    storage
        .ensure_schema(trace_retention_days, spend_log_retention_days)
        .await
        .map_err(|source| SchemaError::Storage { source })
}

mod tests;
