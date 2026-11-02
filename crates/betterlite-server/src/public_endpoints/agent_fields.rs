use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use axum::{Json, Router};

pub mod catalog;
pub mod error;
mod tests;
pub mod types;

use catalog::agent_fields;
use error::AgentFieldsError;
use types::AgentCreateInfo;

pub const AGENT_FIELDS_PATH: &str = "/public/agents/fields";

// Built once in agent_fields_routes and shared through router state, since the bundled file never changes at runtime.
type AgentFieldsState = Arc<Result<Vec<AgentCreateInfo>, AgentFieldsError>>;

pub async fn get_agent_fields(
    State(fields): State<AgentFieldsState>,
) -> Result<Response, AgentFieldsError> {
    match &*fields {
        Ok(response) => Ok(Json(response.clone()).into_response()),
        Err(error) => Err(error.clone()),
    }
}

// Awaits a future B003 integration task that merges this into the server's main router.
pub fn agent_fields_routes() -> Router {
    let fields: AgentFieldsState = Arc::new(agent_fields());
    Router::new()
        .route(AGENT_FIELDS_PATH, get(get_agent_fields))
        .with_state(fields)
}
