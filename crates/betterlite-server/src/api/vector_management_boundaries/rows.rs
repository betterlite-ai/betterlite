use std::future::Future;

use betterlite_db::auxiliary_repository::bindings::ManagedVectorStoresTable as StoresBinding;
use betterlite_db::auxiliary_repository::repository::{PrismaTableRepository, TableError};
use betterlite_db::postgres_store::errors::backend;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use deadpool_postgres::GenericClient as _;

// Prisma's find_unique over the id the management endpoints ask for
const FIND_UNIQUE_SQL: &str = "SELECT to_jsonb(row)::text FROM \"LiteLLM_ManagedVectorStoresTable\" AS row WHERE row.vector_store_id = $1";

// the one repository read the management helpers make, so a test answers it without Postgres
pub trait VectorStoreRows: Send + Sync {
    fn find_unique(
        &self,
        vector_store_id: &str,
    ) -> impl Future<Output = Result<Option<LiteLlmManagedVectorStore>, TableError>> + Send;
}

// B726's repository over B858's binding, read straight into the record LiteLLM dumps the row into
impl VectorStoreRows for PrismaTableRepository<'_, StoresBinding> {
    // _row_to_vector_store: LiteLLM_ManagedVectorStore(**row.model_dump()); a stored JSON string stays a string and is_config stays absent until _with_ownership supplies it
    async fn find_unique(
        &self,
        vector_store_id: &str,
    ) -> Result<Option<LiteLlmManagedVectorStore>, TableError> {
        let access = self.access()?;
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
