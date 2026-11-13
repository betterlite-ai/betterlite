use betterlite_redis::config_sync::publish::PublishChange;
use chrono::{DateTime, Utc};
use deadpool_postgres::GenericClient;
use serde::de::DeserializeOwned;

use crate::postgres_store::errors::backend;

use super::bindings::ManagedObjectTable;
use super::decoder::decode_managed_batch_record;
use super::repository::{PrismaClient, PrismaTableRepository, TableError};

const FIND_ROW_SQL: &str = "SELECT file_object::text, status FROM \"LiteLLM_ManagedObjectTable\" \
     WHERE unified_object_id = $1 LIMIT 1";

const UPDATE_SQL: &str = "UPDATE \"LiteLLM_ManagedObjectTable\" \
     SET file_object = $2::text::jsonb, status = $3, updated_by = $4, updated_at = now() \
     WHERE unified_object_id = $1 \
     AND ($5::text IS NULL OR status = $5) \
     AND ($6::timestamp IS NULL OR updated_at < $6)";

const TOUCH_SQL: &str = "UPDATE \"LiteLLM_ManagedObjectTable\" \
     SET updated_by = $2, updated_at = now() WHERE unified_object_id = $1";

pub struct ManagedBatchRepository<'db> {
    base: PrismaTableRepository<'db, ManagedObjectTable>,
}

// The caller serializes its B1035 batch contract once, so this crate never names that type
pub struct BatchWrite {
    pub id: String,
    pub status: String,
    pub file_object: String,
}

#[derive(Debug, Clone, Default)]
pub struct BatchUnchanged {
    pub unified_object_id: Option<String>,
    pub status: Option<String>,
    pub updated_before: Option<DateTime<Utc>>,
}

struct StoredBatch {
    file_object: String,
    status: Option<String>,
}

impl<'db> ManagedBatchRepository<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, publish: PublishChange<TableError>) -> Self {
        Self {
            base: PrismaTableRepository::new(client, publish, ManagedObjectTable),
        }
    }

    async fn find_row(&self, unified_batch_id: &str) -> Result<Option<StoredBatch>, TableError> {
        let access = self.base.access()?;
        let client = access.read.session().await?;
        let row = client
            .query_opt(FIND_ROW_SQL, &[&unified_batch_id])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let file_object = row
            .try_get(0)
            .map_err(|source| TableError::Database(backend(source)))?;
        let status = row
            .try_get(1)
            .map_err(|source| TableError::Database(backend(source)))?;
        Ok(Some(StoredBatch {
            file_object,
            status,
        }))
    }

    pub async fn load_batch<Batch>(
        &self,
        unified_batch_id: &str,
    ) -> Result<Option<Batch>, TableError>
    where
        Batch: DeserializeOwned,
    {
        let Some(row) = self.find_row(unified_batch_id).await? else {
            return Ok(None);
        };
        Ok(decode_managed_batch_record(row.file_object.as_bytes())?)
    }

    pub async fn load_status(&self, unified_batch_id: &str) -> Result<Option<String>, TableError> {
        Ok(self
            .find_row(unified_batch_id)
            .await?
            .and_then(|row| row.status))
    }

    pub async fn compare_and_set(
        &self,
        batch: &BatchWrite,
        unchanged: &BatchUnchanged,
        updated_by: Option<&str>,
    ) -> Result<bool, TableError> {
        let access = self.base.access()?;
        let client = access.write.session().await?;
        let id = unchanged
            .unified_object_id
            .as_deref()
            .unwrap_or(batch.id.as_str());
        let updated_before = unchanged.updated_before.map(|value| value.naive_utc());
        let rows = client
            .execute(
                UPDATE_SQL,
                &[
                    &id,
                    &batch.file_object,
                    &batch.status,
                    &updated_by,
                    &unchanged.status,
                    &updated_before,
                ],
            )
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        Ok(rows > 0)
    }

    pub async fn touch(
        &self,
        unified_batch_id: &str,
        updated_by: Option<&str>,
    ) -> Result<(), TableError> {
        let access = self.base.access()?;
        let client = access.write.session().await?;
        client
            .execute(TOUCH_SQL, &[&unified_batch_id, &updated_by])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        Ok(())
    }
}
