use std::sync::Arc;

use betterlite_core::api::vector_store_registry::repository::{
    PrismaVectorStoreDatabase, VectorStoreDatabase,
};
use betterlite_db::auxiliary_repository::bindings::ManagedVectorStoresTable as StoresBinding;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_openai::api::vector_store_records::records::pydantic::LiteLlmManagedVectorStoreIndex;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;

use crate::api::vector_management_boundaries::rows::VectorStoreRows;

// the proxy's own Postgres behind the two reads management needs: find_many by created_at and find_unique
pub struct PostgresVectorManagementDatabase {
    store: Arc<PostgresStore>,
    availability: Availability,
}

impl PostgresVectorManagementDatabase {
    #[must_use]
    pub fn new(store: Arc<PostgresStore>) -> Self {
        Self {
            store,
            availability: Availability::default(),
        }
    }

    fn client(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: self.store.as_ref(),
            reader: self.store.as_ref(),
            availability: &self.availability,
        }
    }

    // the registry's two table reads, answered from the read replica like Prisma does
    fn registry(&self) -> PrismaVectorStoreDatabase<'_> {
        PrismaVectorStoreDatabase::new(Some(self.client()), redis_config_change_publisher(None))
    }
}

impl VectorStoreDatabase for PostgresVectorManagementDatabase {
    async fn stores(&self) -> Result<Vec<LiteLlmManagedVectorStore>, TableError> {
        VectorStoreDatabase::stores(&self.registry()).await
    }

    async fn indexes(&self) -> Result<Vec<LiteLlmManagedVectorStoreIndex>, TableError> {
        VectorStoreDatabase::indexes(&self.registry()).await
    }
}

impl VectorStoreRows for PostgresVectorManagementDatabase {
    async fn find_unique(
        &self,
        vector_store_id: &str,
    ) -> Result<Option<LiteLlmManagedVectorStore>, TableError> {
        let repository = PrismaTableRepository::new(
            Some(self.client()),
            redis_config_change_publisher(None),
            StoresBinding,
        );
        VectorStoreRows::find_unique(&repository, vector_store_id).await
    }
}
