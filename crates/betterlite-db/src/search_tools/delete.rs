use super::error::StoreError;
use super::table_client::SearchToolTableClient;

mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedSearchTool {
    pub message: String,
    pub search_tool_name: String,
}

pub async fn remove<Table>(
    search_tool_id: &str,
    table: &Table,
) -> Result<DeletedSearchTool, StoreError>
where
    Table: SearchToolTableClient,
{
    let existing =
        table
            .find_unique(search_tool_id)
            .await?
            .ok_or_else(|| StoreError::NotFound {
                search_tool_id: search_tool_id.to_owned(),
            })?;
    table.delete(search_tool_id).await?;
    Ok(DeletedSearchTool {
        message: format!("Search tool {search_tool_id} deleted successfully"),
        search_tool_name: existing.search_tool_name,
    })
}
