use async_trait::async_trait;
use axum::extract::Extension;
use serde::de::DeserializeOwned;

use crate::auth::auth_identity::AuthContext;
use crate::workflow_events::WorkflowEventsState;
use crate::workflow_events::error::EventsError;
use crate::workflow_events::router::{require_caller, require_ports};
use crate::workflow_events::store::{WorkflowEventStore, WriteError};
use crate::workflow_runs::access::require_run;
use crate::workflow_runs::error::RunsError;
use crate::workflow_runs::models::{MAX_SEQUENCE_RETRIES, SequenceTable};
use crate::workflow_runs::router::decode_body;
use crate::workflow_runs::sequence::next_sequence_number;
use crate::workflow_runs::store::WorkflowRunStore;

#[async_trait]
pub(super) trait PendingWrite {
    type Row;

    fn table(&self) -> SequenceTable;

    async fn write(
        &self,
        store: &dyn WorkflowEventStore,
        run_id: &str,
        sequence_number: i32,
    ) -> Result<Self::Row, WriteError>;
}

// A collision means another writer committed that number, so the next attempt asks for a fresh MAX+1
pub(super) async fn append_with_retry<Pending: PendingWrite + Sync>(
    runs: &dyn WorkflowRunStore,
    store: &dyn WorkflowEventStore,
    run_id: &str,
    pending: &Pending,
) -> Result<Pending::Row, EventsError> {
    let table = pending.table();
    let mut remaining = MAX_SEQUENCE_RETRIES;
    loop {
        let sequence_number = next_sequence_number(runs, run_id, table).await?;
        match pending.write(store, run_id, sequence_number).await {
            Ok(row) => return Ok(row),
            Err(WriteError::Store(source)) => {
                return Err(EventsError::from(RunsError::Store(source)));
            }
            Err(WriteError::SequenceTaken) => {
                remaining = remaining.saturating_sub(1);
                if remaining == 0 {
                    return Err(EventsError::Conflict);
                }
            }
        }
    }
}

// The body decodes before the database is touched, because FastAPI validates it before the handler runs
pub(super) async fn prepare_append<'state, Request>(
    state: &'state WorkflowEventsState,
    identity: Option<Extension<AuthContext>>,
    body: &[u8],
    run_id: &str,
) -> Result<
    (
        Request,
        &'state dyn WorkflowRunStore,
        &'state dyn WorkflowEventStore,
    ),
    EventsError,
>
where
    Request: DeserializeOwned + Send,
{
    let caller = require_caller(identity)?;
    let request: Request = decode_body(body)?;
    let (runs, writes) = require_ports(state)?;
    // a write skips the ownership check only for a full PROXY_ADMIN
    require_run(runs, run_id, Some(&caller))
        .await
        .map_err(EventsError::run_lookup)?;
    Ok((request, runs, writes))
}
