use std::future::Future;

use chrono::{DateTime, Utc};

use crate::auxiliary_repository::repository::TableError;

use super::record::SearchToolRecord;

pub mod postgres;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatedAtOrder {
    Ascending,
    Descending,
}

// The two parameter fields carry JSON text, which the table binds as a JSON document
#[derive(Clone)]
pub struct SearchToolCreate {
    pub search_tool_name: String,
    pub litellm_params: String,
    pub search_tool_info: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct SearchToolUpdate {
    pub search_tool_name: String,
    pub litellm_params: String,
    pub search_tool_info: String,
    pub updated_at: DateTime<Utc>,
}

pub trait SearchToolTableClient: Send + Sync {
    fn create(
        &self,
        data: SearchToolCreate,
    ) -> impl Future<Output = Result<SearchToolRecord, TableError>> + Send;

    fn find_unique(
        &self,
        search_tool_id: &str,
    ) -> impl Future<Output = Result<Option<SearchToolRecord>, TableError>> + Send;

    fn find_many(
        &self,
        order: Option<CreatedAtOrder>,
    ) -> impl Future<Output = Result<Vec<SearchToolRecord>, TableError>> + Send;

    fn update(
        &self,
        search_tool_id: &str,
        data: SearchToolUpdate,
    ) -> impl Future<Output = Result<SearchToolRecord, TableError>> + Send;

    fn delete(
        &self,
        search_tool_id: &str,
    ) -> impl Future<Output = Result<SearchToolRecord, TableError>> + Send;
}
