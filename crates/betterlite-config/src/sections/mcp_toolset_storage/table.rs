use std::future::Future;

use chrono::NaiveDateTime;
use serde::{Deserialize, Deserializer};

use super::{
    MCPToolset, MCPToolsetStorageError, MCPToolsetTool, ToolsetLookup, UpdateMCPToolsetRequest,
    optional_toolset,
};

#[derive(Debug, Clone)]
pub struct ToolsetRow {
    pub toolset_id: String,
    pub toolset_name: String,
    pub description: Option<String>,
    pub tools: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub created_by: Option<String>,
    pub updated_at: Option<NaiveDateTime>,
    pub updated_by: Option<String>,
}

pub enum ToolsetFilter {
    All,
    Ids(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Patch<T> {
    #[default]
    Keep,
    Clear,
    Set(T),
}

pub struct ToolsetUpdate {
    pub toolset_name: Option<String>,
    pub description: Patch<String>,
    pub tools: Option<String>,
    pub updated_by: String,
}

impl ToolsetUpdate {
    pub fn from_request(
        data: &UpdateMCPToolsetRequest,
        touched_by: &str,
    ) -> Result<Self, MCPToolsetStorageError> {
        let toolset_name = match &data.toolset_name {
            Patch::Set(name) => Some(name.clone()),
            Patch::Keep | Patch::Clear => None,
        };
        let tools = match &data.tools {
            Patch::Set(tools) => {
                Some(serde_json::to_string(tools).map_err(MCPToolsetStorageError::Tools)?)
            }
            Patch::Keep | Patch::Clear => None,
        };
        Ok(Self {
            toolset_name,
            description: data.description.clone(),
            tools,
            updated_by: touched_by.to_owned(),
        })
    }
}

pub fn deserialize_patch<'de, D, T>(deserializer: D) -> Result<Patch<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)?
        .map_or_else(|| Ok(Patch::Clear), |value| Ok(Patch::Set(value)))
}

pub trait ToolsetTable: Send + Sync {
    fn create(
        &self,
        row: ToolsetRow,
    ) -> impl Future<Output = Result<ToolsetRow, MCPToolsetStorageError>> + Send;

    fn find_unique(
        &self,
        toolset_id: &str,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, MCPToolsetStorageError>> + Send;

    fn find_first_by_name(
        &self,
        toolset_name: &str,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, MCPToolsetStorageError>> + Send;

    fn find_many(
        &self,
        filter: ToolsetFilter,
        use_writer: bool,
    ) -> impl Future<Output = Result<Vec<ToolsetRow>, MCPToolsetStorageError>> + Send;

    fn update(
        &self,
        toolset_id: &str,
        update: ToolsetUpdate,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, MCPToolsetStorageError>> + Send;

    fn delete(
        &self,
        toolset_id: &str,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, MCPToolsetStorageError>> + Send;

    fn get_mcp_toolset(&self, toolset_id: &str) -> impl Future<Output = ToolsetLookup> + Send {
        let found = self.find_unique(toolset_id);
        async move { optional_toolset(found.await?) }
    }

    fn get_mcp_toolset_by_name(
        &self,
        toolset_name: &str,
    ) -> impl Future<Output = ToolsetLookup> + Send {
        let found = self.find_first_by_name(toolset_name);
        async move { optional_toolset(found.await?) }
    }

    fn delete_mcp_toolset(&self, toolset_id: &str) -> impl Future<Output = ToolsetLookup> + Send {
        let found = self.delete(toolset_id);
        async move { optional_toolset(found.await?) }
    }
}

pub fn toolset_from_row(row: ToolsetRow) -> Result<MCPToolset, MCPToolsetStorageError> {
    let tools = match row.tools {
        Some(text) => serde_json::from_str::<Vec<MCPToolsetTool>>(&text)
            .map_err(MCPToolsetStorageError::Tools)?,
        None => Vec::new(),
    };
    Ok(MCPToolset {
        toolset_id: row.toolset_id,
        toolset_name: row.toolset_name,
        description: row.description,
        tools,
        created_at: row.created_at.as_ref().map(NaiveDateTime::and_utc),
        created_by: row.created_by,
        updated_at: row.updated_at.as_ref().map(NaiveDateTime::and_utc),
        updated_by: row.updated_by,
    })
}
