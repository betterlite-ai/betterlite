use chrono::{DateTime, Utc};

use dump::dump_fields;

use super::convert::tool::SearchTool;
use super::error::{SearchToolsError, StoreError};
use super::record::SearchToolRecord;
use super::table_client::{SearchToolCreate, SearchToolTableClient};

pub mod dump;

mod tests;

pub async fn persist<Table>(
    now: DateTime<Utc>,
    search_tool: &SearchTool,
    table: &Table,
) -> Result<SearchToolRecord, SearchToolsError>
where
    Table: SearchToolTableClient,
{
    let fields = dump_fields(search_tool).map_err(|error| SearchToolsError::Add(error).logged())?;
    table
        .create(SearchToolCreate {
            search_tool_name: search_tool.search_tool_name.clone(),
            litellm_params: fields.litellm_params,
            search_tool_info: fields.search_tool_info,
            created_at: now,
            updated_at: now,
        })
        .await
        .map_err(|error| SearchToolsError::Add(StoreError::Table(error)).logged())
}
