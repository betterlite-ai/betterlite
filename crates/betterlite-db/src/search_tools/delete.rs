use super::error::{SearchToolsError, StoreError};
use super::record::SearchToolRecord;
use super::table_client::SearchToolTableClient;

pub(crate) mod lookup;

mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedSearchTool {
    pub message: String,
    pub search_tool_name: String,
}

pub async fn remove<Table>(
    search_tool_id: &str,
    table: &Table,
) -> Result<SearchToolRecord, SearchToolsError>
where
    Table: SearchToolTableClient,
{
    table
        .delete(search_tool_id)
        .await
        .map_err(|error| SearchToolsError::Delete(StoreError::Table(error)).logged())
}
