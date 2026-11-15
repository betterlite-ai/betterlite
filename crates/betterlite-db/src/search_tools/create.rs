use chrono::{DateTime, Utc};

use dump::dump_fields;

use super::convert::isoformat;
use super::convert::tool::SearchTool;
use super::error::StoreError;
use super::table_client::{SearchToolCreate, SearchToolTableClient};

pub mod dump;

mod tests;

pub async fn persist<Table>(
    now: DateTime<Utc>,
    search_tool: SearchTool,
    table: &Table,
) -> Result<SearchTool, StoreError>
where
    Table: SearchToolTableClient,
{
    let fields = dump_fields(&search_tool)?;
    let created = table
        .create(SearchToolCreate {
            search_tool_name: search_tool.search_tool_name.clone(),
            litellm_params: fields.litellm_params,
            search_tool_info: fields.search_tool_info,
            created_at: now,
            updated_at: now,
        })
        .await?;
    Ok(SearchTool {
        search_tool_id: Some(created.search_tool_id),
        created_at: Some(isoformat(created.created_at)),
        updated_at: Some(isoformat(created.updated_at)),
        ..search_tool
    })
}
