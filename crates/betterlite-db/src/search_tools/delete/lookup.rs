use super::super::error::{SearchToolsError, StoreError};
use super::super::record::SearchToolRecord;
use super::super::table_client::SearchToolTableClient;

// LiteLLM raises the not-found message from the lookup, before it issues the delete
pub(crate) async fn existing<Table>(
    search_tool_id: &str,
    table: &Table,
) -> Result<SearchToolRecord, SearchToolsError>
where
    Table: SearchToolTableClient,
{
    table
        .find_unique(search_tool_id)
        .await
        .map_err(|error| SearchToolsError::Delete(StoreError::Table(error)).logged())?
        .ok_or_else(|| {
            SearchToolsError::Delete(StoreError::NotFound {
                search_tool_id: search_tool_id.to_owned(),
            })
            .logged()
        })
}
