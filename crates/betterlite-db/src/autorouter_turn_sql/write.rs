use deadpool_postgres::GenericClient;
use tokio_postgres::types::ToSql;

use super::sql::upsert_autorouter_session_sql;
use super::transaction::{AutoRouterTurnTransaction, upsert_params};
use crate::postgres_store::errors::{DbError, backend};

#[derive(Debug, thiserror::Error)]
pub enum TurnWriteError {
    #[error("auto-router turn upsert failed")]
    Statement(#[source] DbError),
}

pub async fn write_autorouter_turn<C>(
    client: &C,
    transaction: &AutoRouterTurnTransaction,
) -> Result<(), TurnWriteError>
where
    C: GenericClient + Sync,
{
    let sql = upsert_autorouter_session_sql();
    let params = upsert_params(transaction);
    let refs: Vec<&(dyn ToSql + Sync)> = params
        .iter()
        .map(|value| -> &(dyn ToSql + Sync) { value.as_ref() })
        .collect();
    client
        .execute(sql.as_str(), &refs)
        .await
        .map_err(|source| TurnWriteError::Statement(backend(source)))?;
    Ok(())
}
