use axum::Json;
use axum::response::{IntoResponse as _, Response};

pub mod catalog;
pub mod error;
mod tests;
pub mod types;

use catalog::agent_fields;
use error::AgentFieldsError;

// Mounted by a future B003 integration task at this path, with get_agent_fields as its GET handler.
pub const AGENT_FIELDS_PATH: &str = "/public/agents/fields";

pub fn get_agent_fields() -> Result<Response, AgentFieldsError> {
    match agent_fields() {
        Ok(fields) => Ok(Json(fields).into_response()),
        Err(error) => Err(error),
    }
}
