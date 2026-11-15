use async_trait::async_trait;
use betterlite_db::auxiliary_repository::bindings::ManagedFileTable as ManagedFileTableBinding;
use betterlite_db::auxiliary_repository::repository::{PrismaTableRepository, TableError};
use betterlite_db::managed_resource_rows::files::ManagedFileTable;
use betterlite_db::postgres_store::errors::backend;

use super::ManagedFileLookup;
use super::error::ManagedFileLookupError;

// the source's find_first where flat_model_file_ids has the raw id, bounded to one row
const FIND_BY_FLAT_ID_SQL: &str = "SELECT row_to_json(r)::text FROM \"LiteLLM_ManagedFileTable\" r \
     WHERE $1 = ANY(r.flat_model_file_ids) LIMIT 1";

pub struct ManagedFileRows<'db> {
    pub repository: PrismaTableRepository<'db, ManagedFileTableBinding>,
}

#[async_trait]
impl ManagedFileLookup for ManagedFileRows<'_> {
    type Error = ManagedFileLookupError;

    async fn find_by_flat_model_file_id(
        &self,
        raw_file_id: &str,
    ) -> Result<Option<ManagedFileTable>, ManagedFileLookupError> {
        let access = self
            .repository
            .access()
            .map_err(TableError::Database)
            .map_err(ManagedFileLookupError::Lookup)?;
        let session = access
            .read
            .session()
            .await
            .map_err(TableError::Database)
            .map_err(ManagedFileLookupError::Lookup)?;
        let row = session
            .query_opt(FIND_BY_FLAT_ID_SQL, &[&raw_file_id])
            .await
            .map_err(|source| {
                ManagedFileLookupError::Lookup(TableError::Database(backend(source)))
            })?;
        let Some(found) = row else {
            return Ok(None);
        };
        let text: String = found.try_get(0).map_err(|source| {
            ManagedFileLookupError::Lookup(TableError::Database(backend(source)))
        })?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|source| ManagedFileLookupError::Lookup(TableError::Row(source)))
    }
}
