use std::future::Future;

use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_pyhost::json_values::JsonObject;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use thiserror::Error;

// the Json columns hold the text LiteLLM wrote, while another tool may have written an object
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum ColumnJson {
    Text(String),
    Object(JsonObject),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SubmissionRow {
    pub guardrail_id: String,
    pub guardrail_name: String,
    pub litellm_params: Option<ColumnJson>,
    pub guardrail_info: Option<ColumnJson>,
    pub status: Option<String>,
    pub team_id: Option<String>,
    pub submitted_at: Option<Timestamp>,
    pub reviewed_at: Option<Timestamp>,
    pub created_at: Option<Timestamp>,
    pub updated_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamFilter {
    AnyTeam,
    Teams(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSubmission {
    pub guardrail_name: String,
    pub litellm_params: String,
    pub guardrail_info: String,
    pub status: String,
    pub team_id: String,
    pub submitted_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum SubmissionRowsError {
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("the stored guardrail row could not be read")]
    Row {
        #[source]
        source: serde_json::Error,
    },
    #[error("the guardrail row was not returned by the insert")]
    MissingRow,
}

pub trait SubmissionRows: Send + Sync {
    fn find_by_id(
        &self,
        guardrail_id: &str,
    ) -> impl Future<Output = Result<Option<SubmissionRow>, SubmissionRowsError>> + Send;

    fn find_by_name(
        &self,
        guardrail_name: &str,
    ) -> impl Future<Output = Result<Option<SubmissionRow>, SubmissionRowsError>> + Send;

    fn find_team_rows(
        &self,
        filter: TeamFilter,
    ) -> impl Future<Output = Result<Vec<SubmissionRow>, SubmissionRowsError>> + Send;

    fn create(
        &self,
        submission: NewSubmission,
    ) -> impl Future<Output = Result<SubmissionRow, SubmissionRowsError>> + Send;
}
