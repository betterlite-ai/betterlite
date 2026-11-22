use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

use super::SubmissionsState;
use super::listing::{get_submission, list_submissions};
use super::register::register_guardrail;
use super::rows::SubmissionRows;

// B8668 mounts these routes and owns their HTTP oracle cases
pub fn routes<S>(state: SubmissionsState<S>) -> Router
where
    S: SubmissionRows + 'static,
{
    Router::new()
        .route("/guardrails/register", post(register_guardrail::<S>))
        .route("/guardrails/submissions", get(list_submissions::<S>))
        .route(
            "/guardrails/submissions/{guardrail_id}",
            get(get_submission::<S>),
        )
        .with_state(Arc::new(state))
}
