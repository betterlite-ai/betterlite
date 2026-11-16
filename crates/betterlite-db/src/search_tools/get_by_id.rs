use super::error::{SearchToolsError, StoreError};
use super::record::SearchToolRecord;
use super::table_client::SearchToolTableClient;

mod tests;

pub async fn lookup<Table>(
    search_tool_id: &str,
    table: &Table,
) -> Result<Option<SearchToolRecord>, SearchToolsError>
where
    Table: SearchToolTableClient,
{
    table
        .find_unique(search_tool_id)
        .await
        .map_err(|error| SearchToolsError::Get(StoreError::Table(error)).logged())
}
