use crate::daily_spend_sql::bind::execute;
use deadpool_postgres::GenericClient;

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
    execute(client, &sql, &params)
        .await
        .map_err(|source| TurnWriteError::Statement(backend(source)))?;
    Ok(())
}
