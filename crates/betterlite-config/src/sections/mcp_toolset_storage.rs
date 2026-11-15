use betterlite_db::auxiliary_repository::repository::TableError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use table::{Patch, ToolsetFilter, ToolsetRow, ToolsetTable, ToolsetUpdate, toolset_from_row};

pub mod sql;
pub mod table;
mod tests;

crate::sections::stub_section!();

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MCPToolsetTool {
    pub server_id: String,
    pub tool_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MCPToolset {
    pub toolset_id: String,
    pub toolset_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tools: Vec<MCPToolsetTool>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewMCPToolsetRequest {
    pub toolset_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tools: Vec<MCPToolsetTool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateMCPToolsetRequest {
    pub toolset_id: String,
    #[serde(default, deserialize_with = "table::deserialize_patch")]
    pub toolset_name: Patch<String>,
    #[serde(default, deserialize_with = "table::deserialize_patch")]
    pub description: Patch<String>,
    #[serde(default, deserialize_with = "table::deserialize_patch")]
    pub tools: Patch<Vec<MCPToolsetTool>>,
}

#[derive(Debug, Error)]
pub enum MCPToolsetStorageError {
    #[error(transparent)]
    Table(#[from] TableError),
    #[error("the toolset tools are not valid JSON")]
    Tools(#[source] serde_json::Error),
}

pub type ToolsetLookup = Result<Option<MCPToolset>, MCPToolsetStorageError>;

pub async fn create_mcp_toolset<T>(
    table: &T,
    data: &NewMCPToolsetRequest,
    touched_by: &str,
) -> Result<MCPToolset, MCPToolsetStorageError>
where
    T: ToolsetTable,
{
    let tools = serde_json::to_string(&data.tools).map_err(MCPToolsetStorageError::Tools)?;
    let row = ToolsetRow {
        toolset_id: Uuid::new_v4().to_string(),
        toolset_name: data.toolset_name.clone(),
        description: data.description.clone(),
        tools: Some(tools),
        created_at: None,
        created_by: Some(touched_by.to_owned()),
        updated_at: None,
        updated_by: Some(touched_by.to_owned()),
    };
    toolset_from_row(table.create(row).await?)
}

pub async fn get_mcp_toolset<T>(table: &T, toolset_id: &str) -> ToolsetLookup
where
    T: ToolsetTable,
{
    table
        .find_unique(toolset_id)
        .await?
        .map(toolset_from_row)
        .transpose()
}

pub async fn get_mcp_toolset_by_name<T>(table: &T, name: &str) -> ToolsetLookup
where
    T: ToolsetTable,
{
    table
        .find_first_by_name(name)
        .await?
        .map(toolset_from_row)
        .transpose()
}

pub async fn list_mcp_toolsets<T>(
    table: &T,
    toolset_ids: Option<&[String]>,
    use_writer: bool,
) -> Result<Vec<MCPToolset>, MCPToolsetStorageError>
where
    T: ToolsetTable,
{
    let filter = toolset_ids.map_or(ToolsetFilter::All, |ids| ToolsetFilter::Ids(ids.to_vec()));
    match listed_toolsets(table, filter, use_writer).await {
        Ok(toolsets) => Ok(toolsets),
        Err(error) if !use_writer => {
            tracing::warn!(error = %error, "the toolset list read failed, returning no toolsets");
            Ok(Vec::new())
        }
        Err(error) => Err(error),
    }
}

async fn listed_toolsets<T>(
    table: &T,
    filter: ToolsetFilter,
    use_writer: bool,
) -> Result<Vec<MCPToolset>, MCPToolsetStorageError>
where
    T: ToolsetTable,
{
    table
        .find_many(filter, use_writer)
        .await?
        .into_iter()
        .map(toolset_from_row)
        .collect()
}

pub async fn update_mcp_toolset<T>(
    table: &T,
    data: &UpdateMCPToolsetRequest,
    touched_by: &str,
) -> Result<Option<MCPToolset>, MCPToolsetStorageError>
where
    T: ToolsetTable,
{
    let update = ToolsetUpdate::from_request(data, touched_by)?;
    let stored = table.update(&data.toolset_id, update).await?;
    optional_toolset(stored)
}

pub async fn delete_mcp_toolset<T>(table: &T, toolset_id: &str) -> ToolsetLookup
where
    T: ToolsetTable,
{
    table
        .delete(toolset_id)
        .await?
        .map(toolset_from_row)
        .transpose()
}

fn optional_toolset(row: Option<ToolsetRow>) -> ToolsetLookup {
    let Some(stored) = row else {
        return Ok(None);
    };
    toolset_from_row(stored).map(Some)
}
