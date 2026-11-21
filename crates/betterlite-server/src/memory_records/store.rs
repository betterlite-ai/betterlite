use async_trait::async_trait;
use axum::http::StatusCode;
use betterlite_db::auxiliary_repository::repository::TableError;

use super::caller::MemoryCaller;
use super::error::{MemoryHttpError, not_found_error};
use super::filter::{KeyMatch, MemoryWhere, visibility_filter};
use super::models::MemoryRow;

// the scoped lookup's answer: a missing row is the 404 every route answers, anything else is the store's own failure
#[derive(Debug, thiserror::Error)]
pub enum MemoryLookupError {
    #[error(transparent)]
    Http(#[from] MemoryHttpError),
    #[error(transparent)]
    Rows(#[from] TableError),
}

// memory_endpoints.py _memory_table: the LiteLLM_MemoryTable queries the scoped lookups make
#[async_trait]
pub trait MemoryRows: Send + Sync {
    // find_many(where=filter, take=1, order={"updated_at": "desc"}): the newest matching row
    async fn newest_memory_row(
        &self,
        filter: &MemoryWhere,
    ) -> Result<Option<MemoryRow>, MemoryLookupError>;
}

// memory_endpoints.py _find_memory_for_caller: the caller-visible row for a key, newest first
pub async fn find_memory_for_caller(
    rows: &dyn MemoryRows,
    key: &str,
    caller: &MemoryCaller,
) -> Result<MemoryRow, MemoryLookupError> {
    let mut filter = MemoryWhere::Key {
        key: KeyMatch::Exact(key.to_owned()),
    };
    if let Some(visibility) = visibility_filter(caller) {
        filter = MemoryWhere::AllOf {
            all_of: vec![filter, visibility],
        };
    }
    let Some(row) = rows.newest_memory_row(&filter).await? else {
        return Err(MemoryLookupError::Http(not_found_error(key)));
    };
    Ok(row)
}

// memory_endpoints.py _find_visible_memory_or_none: suppress the 404, propagate every other failure
pub async fn find_visible_memory_or_none(
    rows: &dyn MemoryRows,
    key: &str,
    caller: &MemoryCaller,
) -> Result<Option<MemoryRow>, MemoryLookupError> {
    match find_memory_for_caller(rows, key, caller).await {
        Ok(row) => Ok(Some(row)),
        Err(MemoryLookupError::Http(error)) if error.status == StatusCode::NOT_FOUND => Ok(None),
        Err(error) => Err(error),
    }
}
