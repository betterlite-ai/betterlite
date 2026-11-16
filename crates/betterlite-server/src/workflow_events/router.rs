use axum::Router;
use axum::extract::Extension;
use axum::routing::{get, post};

use crate::auth::auth_identity::AuthContext;
use crate::workflow_events::WorkflowEventsState;
use crate::workflow_events::error::EventsError;
use crate::workflow_events::events::append::append_event;
use crate::workflow_events::events::list::list_events;
use crate::workflow_events::messages::append::append_message;
use crate::workflow_events::messages::list::list_messages;
use crate::workflow_events::store::WorkflowEventStore;
use crate::workflow_runs::access::RunCaller;
use crate::workflow_runs::error::RunsError;
use crate::workflow_runs::store::WorkflowRunStore;

pub fn router(state: WorkflowEventsState) -> Router {
    Router::new()
        .route(
            "/v1/workflows/runs/{run_id}/events",
            post(append_event).get(list_events),
        )
        .route(
            "/v1/workflows/runs/{run_id}/messages",
            post(append_message).get(list_messages),
        )
        .with_state(state)
}

pub(super) fn require_caller(
    identity: Option<Extension<AuthContext>>,
) -> Result<RunCaller, EventsError> {
    let Some(Extension(context)) = identity else {
        return Err(EventsError::from(RunsError::Unauthenticated));
    };
    Ok(RunCaller::from_principal(&context.principal))
}

// Both ports read the same database, so one missing means the database is not connected.
pub(super) fn require_ports(
    state: &WorkflowEventsState,
) -> Result<(&dyn WorkflowRunStore, &dyn WorkflowEventStore), EventsError> {
    let (Some(runs), Some(writes)) = (state.runs.as_deref(), state.events.as_deref()) else {
        return Err(EventsError::from(RunsError::DbNotConnected));
    };
    Ok((runs, writes))
}
