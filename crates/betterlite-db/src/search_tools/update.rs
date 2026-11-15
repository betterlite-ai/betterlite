use chrono::{DateTime, Utc};

use super::convert::SearchToolRow;
use super::convert::tool::SearchTool;
use super::create::dump::dump_fields;
use super::error::StoreError;
use super::registry::SearchToolRegistry;
use super::table_client::{SearchToolTableClient, SearchToolUpdate};

mod tests;

pub async fn replace<Table>(
    search_tool_id: &str,
    search_tool: &SearchTool,
    updated_at: DateTime<Utc>,
    table: &Table,
) -> Result<SearchToolRow, StoreError>
where
    Table: SearchToolTableClient,
{
    let fields = dump_fields(search_tool)?;
    let updated = table
        .update(
            search_tool_id,
            SearchToolUpdate {
                search_tool_name: search_tool.search_tool_name.clone(),
                litellm_params: fields.litellm_params,
                search_tool_info: fields.search_tool_info,
                updated_at,
            },
        )
        .await?;
    Ok(SearchToolRegistry::convert_prisma_to_dict(&updated))
}
