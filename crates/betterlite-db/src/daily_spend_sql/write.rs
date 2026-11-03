use super::{
    bind::execute,
    builder::build_bulk_upsert,
    entities::{DailySpendEntity, SpendRow},
    merge::merge_by_conflict_key,
    values::DailySpendValueError,
};
use crate::postgres_store::errors::{DbError, backend};
use deadpool_postgres::GenericClient;

#[derive(Debug, thiserror::Error)]
pub enum DailyWriteError {
    #[error("invalid daily spend input")]
    Input(#[from] DailySpendValueError),
    #[error("daily spend upsert failed")]
    Statement(#[source] DbError),
}
pub async fn write_bulk_upsert<C, G>(
    client: &C,
    entity: DailySpendEntity,
    rows: &[SpendRow],
    generate_id: G,
) -> Result<(), DailyWriteError>
where
    C: GenericClient + Sync,
    G: FnMut() -> uuid::Uuid,
{
    if rows.is_empty() {
        return Ok(());
    }
    let table = entity.table();
    let groups = merge_by_conflict_key(&table, rows)?;
    let (sql, params) = build_bulk_upsert(&table, &groups, generate_id)?;
    execute(client, &sql, &params)
        .await
        .map_err(|error| DailyWriteError::Statement(backend(error)))?;
    Ok(())
}
