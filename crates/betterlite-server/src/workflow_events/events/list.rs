use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::Uri;

use crate::auth::auth_identity::AuthContext;
use crate::workflow_events::WorkflowEventsState;
use crate::workflow_events::error::EventsError;
use crate::workflow_events::models::EventListResponse;
use crate::workflow_events::read::prepare_list;

pub(in crate::workflow_events) async fn list_events(
    State(state): State<WorkflowEventsState>,
    identity: Option<Extension<AuthContext>>,
    Path(run_id): Path<String>,
    uri: Uri,
) -> Result<Json<EventListResponse>, EventsError> {
    let (take, writes) = prepare_list(&state, identity, &uri, &run_id).await?;
    let events = writes.events_for_run(&run_id, take).await?;
    let count = events.len();
    Ok(Json(EventListResponse { events, count }))
}
