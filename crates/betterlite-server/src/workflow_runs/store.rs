use async_trait::async_trait;
use thiserror::Error;

use crate::workflow_runs::models::{
    RunCreateData, RunUpdateData, RunWhere, SequenceRow, SequenceTable, WorkflowRunRow,
};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("workflow table access failed: {message}")]
    Failed { message: String },
}

#[async_trait]
pub trait WorkflowRunStore: Send + Sync {
    async fn create(&self, data: &RunCreateData) -> Result<WorkflowRunRow, StoreError>;

    // newest first by created_at, at most `take` rows
    async fn find_many(
        &self,
        filter: &RunWhere,
        take: u8,
    ) -> Result<Vec<WorkflowRunRow>, StoreError>;

    async fn find_unique(&self, run_id: &str) -> Result<Option<WorkflowRunRow>, StoreError>;

    // the row with `events` holding only the event of the highest sequence_number
    async fn find_unique_with_latest_event(
        &self,
        run_id: &str,
    ) -> Result<Option<WorkflowRunRow>, StoreError>;

    // None when no row has this run_id
    async fn update(
        &self,
        run_id: &str,
        data: &RunUpdateData,
    ) -> Result<Option<WorkflowRunRow>, StoreError>;

    // the row of the highest sequence_number for the run in the events or messages table
    async fn latest_sequence(
        &self,
        run_id: &str,
        table: SequenceTable,
    ) -> Result<Option<SequenceRow>, StoreError>;
}
