use super::convert::tool::SearchTool;
use super::error::StoreError;
use super::registry::SearchToolRegistry;
use super::table_client::SearchToolTableClient;

mod tests;

pub async fn lookup<Table>(
    search_tool_id: &str,
    table: &Table,
) -> Result<Option<SearchTool>, StoreError>
where
    Table: SearchToolTableClient,
{
    let Some(record) = table.find_unique(search_tool_id).await? else {
        return Ok(None);
    };
    let row = SearchToolRegistry::convert_prisma_to_dict(&record);
    Ok(Some(SearchTool::try_from(row)?))
}
