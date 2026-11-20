use std::future::Future;
use std::sync::Arc;

use betterlite_db::auxiliary_repository::bindings::ManagedVectorStoresTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::postgres_store::errors::backend;
use betterlite_openai::api::vector_store_records::records::pydantic::LiteLlmManagedVectorStoreIndex;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_redis::config_sync::publish::PublishChange;
use deadpool_postgres::GenericClient as _;

use crate::api::vector_store_registry::repository::{
    PrismaVectorStoreDatabase, VectorStoreDatabase,
};

// find_unique(where={"vector_store_id": id}): the id is the table's primary key
const FIND_STORE_SQL: &str = "SELECT to_jsonb(row)::text FROM \"LiteLLM_ManagedVectorStoresTable\" AS row WHERE row.vector_store_id = $1";

// the extra read the consuming fallback needs on top of B1062's table reads
pub trait VectorStoreSelectionDatabase: VectorStoreDatabase {
    fn find_store(
        &self,
        vector_store_id: &str,
    ) -> impl Future<Output = Result<Option<LiteLlmManagedVectorStore>, TableError>> + Send;
}

pub struct PrismaVectorStoreSelection<'db> {
    stores: PrismaTableRepository<'db, ManagedVectorStoresTable>,
    base: PrismaVectorStoreDatabase<'db>,
}

impl<'db> PrismaVectorStoreSelection<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, publish: PublishChange<TableError>) -> Self {
        Self {
            stores: PrismaTableRepository::new(
                client,
                Arc::clone(&publish),
                ManagedVectorStoresTable,
            ),
            base: PrismaVectorStoreDatabase::new(client, publish),
        }
    }
}

impl VectorStoreDatabase for PrismaVectorStoreSelection<'_> {
    async fn stores(&self) -> Result<Vec<LiteLlmManagedVectorStore>, TableError> {
        self.base.stores().await
    }

    async fn indexes(&self) -> Result<Vec<LiteLlmManagedVectorStoreIndex>, TableError> {
        self.base.indexes().await
    }
}

impl VectorStoreSelectionDatabase for PrismaVectorStoreSelection<'_> {
    async fn find_store(
        &self,
        vector_store_id: &str,
    ) -> Result<Option<LiteLlmManagedVectorStore>, TableError> {
        let access = self.stores.access()?;
        let client = access.read.session().await?;
        let row = client
            .query_opt(FIND_STORE_SQL, &[&vector_store_id])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        let Some(found) = row else {
            return Ok(None);
        };
        let text: String = found
            .try_get(0)
            .map_err(|source| TableError::Database(backend(source)))?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(TableError::Row)
    }
}
