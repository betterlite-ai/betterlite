use super::diagnostics::PoolFailure;
use super::errors::DbError;
use deadpool_postgres::{Client, Pool, Transaction};

#[derive(Clone)]
pub struct PostgresStore {
    pub pool: Option<Pool>,
}

// Borrowing the pooled client keeps it checked out until the driver's rollback guard has run.
pub type DbSession<'client> = Transaction<'client>;

impl PostgresStore {
    pub async fn session(&self) -> Result<Client, DbError> {
        let pool = self.pool.as_ref().ok_or(DbError::NoDbConnected)?;
        pool.get().await.map_err(|driver| DbError::Pool {
            source: PoolFailure { driver },
        })
    }
}
