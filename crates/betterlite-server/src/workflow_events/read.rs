use axum::extract::{Extension, Query};
use axum::http::Uri;
use serde::Deserialize;

use crate::auth::auth_identity::AuthContext;
use crate::workflow_events::WorkflowEventsState;
use crate::workflow_events::error::{EventsError, LimitIssue};
use crate::workflow_events::router::{require_caller, require_ports};
use crate::workflow_events::store::WorkflowEventStore;
use crate::workflow_runs::access::require_run;
use crate::workflow_runs::error::{RunsError, ValidationFailure};

const DEFAULT_LIMIT: u16 = 100;
const MAX_LIMIT: u16 = 500;

#[derive(Debug, Deserialize)]
struct LimitQuery {
    limit: Option<String>,
}

fn parse_limit(raw: Option<&str>) -> Result<u16, EventsError> {
    let Some(text) = raw else {
        return Ok(DEFAULT_LIMIT);
    };
    match text.parse::<i128>() {
        Err(_error) => Err(EventsError::from(LimitIssue::NotInteger)),
        Ok(limit) if limit < 1 => Err(EventsError::from(LimitIssue::TooSmall)),
        Ok(limit) => u16::try_from(limit)
            .ok()
            .filter(|bounded| *bounded <= MAX_LIMIT)
            .ok_or(EventsError::from(LimitIssue::TooLarge)),
    }
}

fn query_limit(uri: &Uri) -> Result<u16, EventsError> {
    let Ok(Query(query)) = Query::<LimitQuery>::try_from_uri(uri) else {
        return Err(EventsError::from(RunsError::Validation(
            ValidationFailure::Query,
        )));
    };
    parse_limit(query.limit.as_deref())
}

// An admin view reads every run; everyone else reads only the runs their own key created
pub(super) async fn prepare_list<'state>(
    state: &'state WorkflowEventsState,
    identity: Option<Extension<AuthContext>>,
    uri: &Uri,
    run_id: &str,
) -> Result<(u16, &'state dyn WorkflowEventStore), EventsError> {
    let caller = require_caller(identity)?;
    let take = query_limit(uri)?;
    let (runs, writes) = require_ports(state)?;
    require_run(runs, run_id, caller.read_scope()).await?;
    Ok((take, writes))
}
