use crate::auxiliary_repository::repository::TableError;
use crate::database_faults::DbError;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Retry(#[from] DbError),
    #[error("Search tool with ID {search_tool_id} not found")]
    NotFound { search_tool_id: String },
    #[error("search tool field {field} is not valid: {source}")]
    Field {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum SearchToolsError {
    #[error("Error adding search tool to DB: {0}")]
    Add(#[source] StoreError),
    #[error("Error deleting search tool from DB: {0}")]
    Delete(#[source] StoreError),
    #[error("Error updating search tool in DB: {0}")]
    Update(#[source] StoreError),
    #[error("Error getting search tools from DB: {0}")]
    List(#[source] StoreError),
    #[error("Error getting search tool from DB: {0}")]
    Get(#[source] StoreError),
}

impl SearchToolsError {
    // LiteLLM logs the exception in the operation that raises it, before the prefixed re-raise
    #[must_use]
    pub(super) fn logged(self) -> Self {
        tracing::error!("{self}");
        self
    }
}
