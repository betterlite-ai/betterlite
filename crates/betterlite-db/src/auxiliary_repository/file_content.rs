use betterlite_redis::config_sync::publish::PublishChange;
use deadpool_postgres::GenericClient;
use uuid::Uuid;

use crate::postgres_store::errors::backend;

use super::repository::{PrismaClient, PrismaTableRepository, TableBinding, TableError};

const INSERT_SQL: &str =
    "INSERT INTO \"LiteLLM_ManagedFileContentTable\" (id, content) VALUES ($1, $2)";
const SELECT_SQL: &str = "SELECT content FROM \"LiteLLM_ManagedFileContentTable\" WHERE id = $1";
const DELETE_SQL: &str = "DELETE FROM \"LiteLLM_ManagedFileContentTable\" WHERE id = $1";

pub struct ManagedFileContentTable;

impl TableBinding for ManagedFileContentTable {
    const NAME: &'static str = "litellm_managedfilecontenttable";
    const TABLE: &'static str = "LiteLLM_ManagedFileContentTable";
}

// LiteLLM_ManagedFileContentTable is outside B4144's allowlist, so its writes never publish
pub struct ManagedFileContentRepository<'db> {
    base: PrismaTableRepository<'db, ManagedFileContentTable>,
}

impl<'db> ManagedFileContentRepository<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, publish: PublishChange<TableError>) -> Self {
        Self {
            base: PrismaTableRepository::new(client, publish, ManagedFileContentTable),
        }
    }

    pub async fn store(&self, content: &[u8]) -> Result<String, TableError> {
        let access = self.base.access()?;
        let client = access.write.session().await?;
        let row_id = Uuid::new_v4().to_string();
        client
            .execute(INSERT_SQL, &[&row_id, &content])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        Ok(row_id)
    }

    pub async fn load(&self, row_id: &str) -> Result<Option<Vec<u8>>, TableError> {
        let access = self.base.access()?;
        let client = access.read.session().await?;
        let row = client
            .query_opt(SELECT_SQL, &[&row_id])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        let Some(row) = row else {
            return Ok(None);
        };
        row.try_get(0)
            .map(Some)
            .map_err(|source| TableError::Database(backend(source)))
    }

    pub async fn delete(&self, row_id: &str) -> Result<(), TableError> {
        let access = self.base.access()?;
        let client = access.write.session().await?;
        client
            .execute(DELETE_SQL, &[&row_id])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        Ok(())
    }
}
