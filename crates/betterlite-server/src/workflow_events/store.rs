use async_trait::async_trait;
use thiserror::Error;

use crate::workflow_events::models::{EventCreateData, MessageCreateData};
use crate::workflow_runs::models::{WorkflowEventRow, WorkflowMessageRow, WorkflowRunStatus};
use crate::workflow_runs::store::StoreError;

#[derive(Debug, Error)]
pub enum WriteError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("a concurrent append took the sequence number this write asked for")]
    SequenceTaken,
}

#[async_trait]
pub trait WorkflowEventStore: Send + Sync {
    // the event insert and the run status change share one transaction, so neither lands alone
    async fn append_event(
        &self,
        data: &EventCreateData,
        new_status: Option<WorkflowRunStatus>,
    ) -> Result<WorkflowEventRow, WriteError>;

    async fn append_message(
        &self,
        data: &MessageCreateData,
    ) -> Result<WorkflowMessageRow, WriteError>;

    // both reads come back ascending by sequence_number, at most `take` rows
    async fn events_for_run(
        &self,
        run_id: &str,
        take: u16,
    ) -> Result<Vec<WorkflowEventRow>, StoreError>;

    async fn messages_for_run(
        &self,
        run_id: &str,
        take: u16,
    ) -> Result<Vec<WorkflowMessageRow>, StoreError>;
}
