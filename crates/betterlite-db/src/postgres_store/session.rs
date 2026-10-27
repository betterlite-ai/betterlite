use super::errors::DbError;
use deadpool_postgres::{Client, Pool};

#[derive(Clone)]
pub struct PostgresStore {
    pub pool: Option<Pool>,
}

// The driver's transaction guard queues rollback on drop, including cancellation during BEGIN.
pub struct DbSession {
    client: Client,
    in_transaction: bool,
}

impl PostgresStore {
    #[must_use]
    pub const fn new(pool: Pool) -> Self {
        let configured_pool = Some(pool);
        Self {
            pool: configured_pool,
        }
    }

    #[must_use]
    pub const fn without_client() -> Self {
        let missing_pool = None;
        Self { pool: missing_pool }
    }

    pub async fn session(&self) -> Result<DbSession, DbError> {
        let pool = self.pool.as_ref().ok_or(DbError::NoDbConnected)?;
        let client = pool
            .get()
            .await
            .map_err(|source| DbError::Pool { source })?;
        Ok(DbSession {
            client,
            in_transaction: false,
        })
    }

    pub async fn transaction(&self) -> Result<DbSession, DbError> {
        let session = self.session().await?;
        session.begin().await
    }
}

impl DbSession {
    pub async fn begin(mut self) -> Result<Self, DbError> {
        self.client
            .batch_execute("BEGIN")
            .await
            .map_err(super::errors::backend)?;
        self.in_transaction = true;
        Ok(self)
    }

    pub async fn commit(self) -> Result<(), DbError> {
        let operation = self.finish("COMMIT");
        operation.await
    }

    pub async fn rollback(self) -> Result<(), DbError> {
        let operation = self.finish("ROLLBACK");
        operation.await
    }

    async fn finish(mut self, command: &str) -> Result<(), DbError> {
        if self.in_transaction {
            self.client
                .batch_execute(command)
                .await
                .map_err(super::errors::backend)?;
            self.in_transaction = false;
        }
        Ok(())
    }
}
