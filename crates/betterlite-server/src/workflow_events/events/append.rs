use async_trait::async_trait;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};

use crate::auth::auth_identity::AuthContext;
use crate::workflow_events::WorkflowEventsState;
use crate::workflow_events::error::EventsError;
use crate::workflow_events::models::{EventCreateData, WorkflowEventCreateRequest};
use crate::workflow_events::store::{WorkflowEventStore, WriteError};
use crate::workflow_events::write::{PendingWrite, append_with_retry, prepare_append};
use crate::workflow_runs::models::{
    EVENT_STATUS_MAP, SequenceTable, WorkflowEventRow, WorkflowRunStatus,
};

struct PendingEvent<'request> {
    request: &'request WorkflowEventCreateRequest,
    new_status: Option<WorkflowRunStatus>,
    encoded_data: Option<String>,
}

#[async_trait]
impl PendingWrite for PendingEvent<'_> {
    type Row = WorkflowEventRow;

    fn table(&self) -> SequenceTable {
        SequenceTable::Events
    }

    async fn write(
        &self,
        store: &dyn WorkflowEventStore,
        run_id: &str,
        sequence_number: i32,
    ) -> Result<WorkflowEventRow, WriteError> {
        let data = EventCreateData {
            run_id: run_id.to_owned(),
            event_type: self.request.event_type.clone(),
            step_name: self.request.step_name.clone(),
            sequence_number,
            data: self.encoded_data.clone(),
        };
        store.append_event(&data, self.new_status).await
    }
}

pub(in crate::workflow_events) async fn append_event(
    State(state): State<WorkflowEventsState>,
    identity: Option<Extension<AuthContext>>,
    Path(run_id): Path<String>,
    body: Bytes,
) -> Result<Json<WorkflowEventRow>, EventsError> {
    let (request, runs, writes) =
        prepare_append::<WorkflowEventCreateRequest>(&state, identity, &body, &run_id).await?;
    let new_status = EVENT_STATUS_MAP
        .iter()
        .find(|(event_type, _status)| *event_type == request.event_type.as_str())
        .map(|(_event_type, status)| *status);
    let pending = PendingEvent {
        request: &request,
        new_status,
        encoded_data: request
            .data
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?,
    };
    let row = append_with_retry(runs, writes, &run_id, &pending).await?;
    Ok(Json(row))
}
