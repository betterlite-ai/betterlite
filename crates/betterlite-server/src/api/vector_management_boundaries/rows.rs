use std::future::Future;

use betterlite_db::auxiliary_repository::bindings::ManagedVectorStoresTable as StoresBinding;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::managed_resource_rows::files::ManagedVectorStoresTable;
use betterlite_db::postgres_store::errors::backend;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_redis::config_sync::publish::PublishChange;
use deadpool_postgres::GenericClient as _;

// Prisma's find_unique over the id the management endpoints ask for
const FIND_UNIQUE_SQL: &str = "SELECT to_jsonb(row)::text FROM \"LiteLLM_ManagedVectorStoresTable\" AS row WHERE row.vector_store_id = $1";

// the one repository read the management helpers make, so a test answers it without Postgres
pub trait VectorStoreRows: Send + Sync {
    fn find_unique(
        &self,
        vector_store_id: &str,
    ) -> impl Future<Output = Result<Option<ManagedVectorStoresTable>, TableError>> + Send;
}

// _row_to_vector_store: LiteLLM_ManagedVectorStore(**row.model_dump()); the table has no is_config column, so the key stays absent until _with_ownership supplies it
pub fn row_to_vector_store(
    row: ManagedVectorStoresTable,
) -> Result<LiteLlmManagedVectorStore, serde_json::Error> {
    let mut dumped = serde_json::to_value(row)?;
    if let Some(fields) = dumped.as_object_mut() {
        fields.remove("is_config");
    }
    serde_json::from_value(dumped)
}

// B726's repository over B858's row, bound to the table the binding names
pub struct ManagedVectorStoreTable<'db> {
    table: PrismaTableRepository<'db, StoresBinding>,
}

impl<'db> ManagedVectorStoreTable<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, publish: PublishChange<TableError>) -> Self {
        Self {
            table: PrismaTableRepository::new(client, publish, StoresBinding),
        }
    }
}

impl VectorStoreRows for ManagedVectorStoreTable<'_> {
    async fn find_unique(
        &self,
        vector_store_id: &str,
    ) -> Result<Option<ManagedVectorStoresTable>, TableError> {
        let access = self.table.access()?;
        let client = access.read.session().await?;
        let found = client
            .query_opt(FIND_UNIQUE_SQL, &[&vector_store_id])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        let Some(row) = found else {
            return Ok(None);
        };
        let text: String = row
            .try_get(0)
            .map_err(|source| TableError::Database(backend(source)))?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(TableError::Row)
    }
}
