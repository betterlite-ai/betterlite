use std::error::Error;
use std::future::Future;

use thiserror::Error;

use super::rows::{PromptRow, PromptRowData, PromptRowJson};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptColumn {
    Id,
    PromptId,
    Version,
    Environment,
    CreatedBy,
    CreatedAt,
    UpdatedAt,
    LiteLLMParams,
    PromptInfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptRowOrder {
    pub column: PromptColumn,
    pub descending: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptRowFilter {
    pub id: Option<String>,
    pub prompt_id: Option<String>,
    pub version: Option<i64>,
    pub environment: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptRowQuery {
    pub filter: PromptRowFilter,
    pub order: Vec<PromptRowOrder>,
    pub take: Option<i64>,
    pub distinct: Vec<PromptColumn>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptRowUpdate {
    pub litellm_params: PromptRowJson,
    pub prompt_info: PromptRowJson,
    pub created_by: Option<String>,
}

pub trait PromptTableActions: Send + Sync {
    type Failure: Error + Send + Sync + 'static;

    fn find_many(
        &self,
        query: PromptRowQuery,
    ) -> impl Future<Output = Result<Vec<PromptRow>, Self::Failure>> + Send;

    fn create(
        &self,
        data: PromptRowData,
    ) -> impl Future<Output = Result<PromptRow, Self::Failure>> + Send;

    fn update(
        &self,
        filter: PromptRowFilter,
        data: PromptRowUpdate,
    ) -> impl Future<Output = Result<Option<PromptRow>, Self::Failure>> + Send;

    fn delete_many(
        &self,
        filter: PromptRowFilter,
    ) -> impl Future<Output = Result<u64, Self::Failure>> + Send;
}

#[derive(Debug, Error)]
pub enum PromptVersionError<TableError>
where
    TableError: Error + 'static,
{
    #[error("the prompt table query failed")]
    Table {
        #[source]
        source: TableError,
    },
    #[error("the prompt version cannot be incremented")]
    VersionOverflow,
}

pub async fn get_next_version_for_prompt<Table>(
    table: &Table,
    prompt_id: &str,
    environment: &str,
) -> Result<i64, PromptVersionError<Table::Failure>>
where
    Table: PromptTableActions,
{
    let prompts = table
        .find_many(PromptRowQuery {
            filter: PromptRowFilter {
                id: None,
                prompt_id: Some(prompt_id.to_owned()),
                version: None,
                environment: Some(environment.to_owned()),
            },
            order: Vec::new(),
            take: None,
            distinct: Vec::new(),
        })
        .await
        .map_err(|source| PromptVersionError::Table { source })?;
    let Some(max_version) = prompts.iter().map(|row| row.version).max() else {
        return Ok(1);
    };
    max_version
        .checked_add(1)
        .ok_or(PromptVersionError::VersionOverflow)
}
