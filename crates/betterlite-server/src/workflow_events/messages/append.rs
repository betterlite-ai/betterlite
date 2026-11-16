use async_trait::async_trait;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};

use crate::auth::auth_identity::AuthContext;
use crate::workflow_events::WorkflowEventsState;
use crate::workflow_events::error::EventsError;
use crate::workflow_events::models::{MessageCreateData, WorkflowMessageCreateRequest};
use crate::workflow_events::store::{WorkflowEventStore, WriteError};
use crate::workflow_events::write::{PendingWrite, append_with_retry, prepare_append};
use crate::workflow_runs::models::{SequenceTable, WorkflowMessageRow};

struct PendingMessage<'request> {
    request: &'request WorkflowMessageCreateRequest,
}

#[async_trait]
impl PendingWrite for PendingMessage<'_> {
    type Row = WorkflowMessageRow;

    fn table(&self) -> SequenceTable {
        SequenceTable::Messages
    }

    async fn write(
        &self,
        store: &dyn WorkflowEventStore,
        run_id: &str,
        sequence_number: i32,
    ) -> Result<WorkflowMessageRow, WriteError> {
        let data = MessageCreateData {
            run_id: run_id.to_owned(),
            role: self.request.role.clone(),
            content: self.request.content.clone(),
            sequence_number,
            session_id: self.request.session_id.clone(),
        };
        store.append_message(&data).await
    }
}

pub(in crate::workflow_events) async fn append_message(
    State(state): State<WorkflowEventsState>,
    identity: Option<Extension<AuthContext>>,
    Path(run_id): Path<String>,
    body: Bytes,
) -> Result<Json<WorkflowMessageRow>, EventsError> {
    let (request, runs, writes) = prepare_append(&state, identity, &body, &run_id).await?;
    let pending = PendingMessage { request: &request };
    let row = append_with_retry(runs, writes, &run_id, &pending).await?;
    Ok(Json(row))
}
