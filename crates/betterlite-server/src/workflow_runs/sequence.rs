use crate::workflow_runs::error::RunsError;
use crate::workflow_runs::models::SequenceTable;
use crate::workflow_runs::store::WorkflowRunStore;

pub async fn next_sequence_number(
    store: &dyn WorkflowRunStore,
    run_id: &str,
    table: SequenceTable,
) -> Result<i32, RunsError> {
    store
        .latest_sequence(run_id, table)
        .await?
        .map_or(Ok(0_i32), |row| {
            row.sequence_number
                .checked_add(1)
                .ok_or(RunsError::SequenceOverflow)
        })
}
