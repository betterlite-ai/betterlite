use std::future::Future;

use betterlite_db::auxiliary_repository::repository::TableError;
use chrono::NaiveDateTime;
use serde::{Deserialize, Deserializer};

use super::{MCPToolset, MCPToolsetStorageError, MCPToolsetTool};

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

pub struct ToolsetUpdate {
    pub toolset_name: Option<String>,
    pub description: Patch<String>,
    pub tools: Option<String>,
    pub updated_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Patch<T> {
    #[default]
    Keep,
    Clear,
    Set(T),
}

pub fn deserialize_patch<'de, D, T>(deserializer: D) -> Result<Patch<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.map_or(Patch::Clear, Patch::Set))
}

pub trait ToolsetTable: Send + Sync {
    fn create(
        &self,
        row: ToolsetRow,
    ) -> impl Future<Output = Result<ToolsetRow, TableError>> + Send;

    fn find_unique(
        &self,
        toolset_id: &str,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, TableError>> + Send;

    fn find_first_by_name(
        &self,
        toolset_name: &str,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, TableError>> + Send;

    fn find_many(
        &self,
        filter: ToolsetFilter,
        use_writer: bool,
    ) -> impl Future<Output = Result<Vec<ToolsetRow>, TableError>> + Send;

    fn update(
        &self,
        toolset_id: &str,
        update: ToolsetUpdate,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, TableError>> + Send;

    fn delete(
        &self,
        toolset_id: &str,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, TableError>> + Send;
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
        created_at: row.created_at,
        created_by: row.created_by,
        updated_at: row.updated_at,
        updated_by: row.updated_by,
    })
}
