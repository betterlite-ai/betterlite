use super::errors::{DbError, backend};
use deadpool_postgres::{Client, Pool};
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

#[derive(Clone)]
pub struct PostgresStore {
    pool: Option<Pool>,
}

impl PostgresStore {
    #[must_use]
    pub const fn new(pool: Pool) -> Self {
        Self { pool: Some(pool) }
    }
    #[must_use]
    pub const fn without_client() -> Self {
        Self { pool: None }
    }
    pub async fn session(&self) -> Result<DbSession, DbError> {
        let pool = self.pool.as_ref().ok_or(DbError::NoDbConnected)?;
        let client = pool
            .get()
            .await
            .map_err(|source| DbError::Pool { source })?;
        Ok(DbSession {
            client,
            transaction: false,
        })
    }

    pub async fn transaction(&self) -> Result<DbSession, DbError> {
        let session = self.session().await?;
        session.begin().await
    }
}

pub struct DbSession {
    client: Client,
    transaction: bool,
}

impl DbSession {
    pub async fn begin(mut self) -> Result<Self, DbError> {
        self.client.batch_execute("BEGIN").await.map_err(backend)?;
        self.transaction = true;
        Ok(self)
    }
    pub async fn commit(mut self) -> Result<(), DbError> {
        if self.transaction {
            self.client.batch_execute("COMMIT").await.map_err(backend)?;
            self.transaction = false;
        }
        Ok(())
    }
    pub async fn rollback(mut self) -> Result<(), DbError> {
        if self.transaction {
            self.client
                .batch_execute("ROLLBACK")
                .await
                .map_err(backend)?;
            self.transaction = false;
        }
        Ok(())
    }
    pub async fn execute(
        &self,
        statement: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, DbError> {
        self.client
            .execute(statement, parameters)
            .await
            .map_err(backend)
    }
    pub async fn query(
        &self,
        statement: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, DbError> {
        self.client
            .query(statement, parameters)
            .await
            .map_err(backend)
    }
    pub async fn query_opt(
        &self,
        statement: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, DbError> {
        self.client
            .query_opt(statement, parameters)
            .await
            .map_err(backend)
    }
    pub async fn query_one(
        &self,
        statement: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, DbError> {
        self.client
            .query_one(statement, parameters)
            .await
            .map_err(backend)
    }
}
