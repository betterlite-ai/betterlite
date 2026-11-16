use crate::workflow_runs::error::RunsError;
use crate::workflow_runs::models::SequenceTable;
use crate::workflow_runs::store::WorkflowRunStore;

pub async fn next_sequence_number(
    store: &dyn WorkflowRunStore,
    run_id: &str,
    table: SequenceTable,
) -> Result<i32, RunsError> {
    match store.latest_sequence(run_id, table).await? {
        Some(row) => row
            .sequence_number
            .checked_add(1)
            .ok_or(RunsError::SequenceOverflow),
        None => Ok(0),
    }
}
