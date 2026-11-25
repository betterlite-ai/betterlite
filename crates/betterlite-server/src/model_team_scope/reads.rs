use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::ResourceRowsError;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_db::user_repository::RowDecodeError;

use crate::model_group_data::errors::GroupDataError;

#[derive(Debug, thiserror::Error)]
pub enum UserReadError {
    #[error(transparent)]
    Session(#[from] DbError),
    #[error(transparent)]
    Row(#[from] DbError<RowDecodeError>),
}

#[derive(Debug, thiserror::Error)]
pub enum ModelRowReadError {
    #[error(transparent)]
    Session(#[from] DbError),
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Row(#[from] ResourceRowsError),
}

// No variant reaches the client: each caller logs the failure and answers as LiteLLM does.
#[derive(Debug, thiserror::Error)]
pub enum TeamScopeReadError {
    #[error(transparent)]
    User(Box<UserReadError>),
    #[error(transparent)]
    Team(Box<TeamQueryError>),
    #[error(transparent)]
    Models(Box<GroupDataError>),
    #[error(transparent)]
    ModelRow(Box<ModelRowReadError>),
}

impl From<UserReadError> for TeamScopeReadError {
    fn from(source: UserReadError) -> Self {
        Self::User(Box::new(source))
    }
}

impl From<TeamQueryError> for TeamScopeReadError {
    fn from(source: TeamQueryError) -> Self {
        Self::Team(Box::new(source))
    }
}

impl From<GroupDataError> for TeamScopeReadError {
    fn from(source: GroupDataError) -> Self {
        Self::Models(Box::new(source))
    }
}

impl From<ModelRowReadError> for TeamScopeReadError {
    fn from(source: ModelRowReadError) -> Self {
        Self::ModelRow(Box::new(source))
    }
}

#[async_trait]
pub trait TeamScopeReads: Send + Sync {
    async fn user_teams(&self, user_id: &str) -> Result<Option<Vec<String>>, TeamScopeReadError>;
    async fn team_models(&self, team_id: &str) -> Result<Option<Vec<String>>, TeamScopeReadError>;
    async fn model_ids_by_names(&self, names: &[String])
    -> Result<Vec<String>, TeamScopeReadError>;
    async fn model_row(
        &self,
        model_id: &str,
    ) -> Result<Option<ProxyModelTable>, TeamScopeReadError>;
}
