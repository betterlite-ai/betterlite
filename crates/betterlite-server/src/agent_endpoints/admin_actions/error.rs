use crate::agent_endpoints::identity_admin::error::{Detail, IdentityAdminError};

use super::ports::PortFailure;

// LiteLLM wraps each route body in `except Exception as e: raise HTTPException(status_code=500, detail=str(e))`
pub(super) fn unexpected_failure(failure: PortFailure) -> IdentityAdminError {
    IdentityAdminError {
        status: 500,
        detail: Detail::Text(failure.to_string()),
    }
}

pub(super) fn missing_agent(agent_id: &str) -> IdentityAdminError {
    IdentityAdminError {
        status: 404,
        detail: Detail::Text(format!("Agent with ID {agent_id} not found")),
    }
}

pub(super) fn unexpected_role() -> IdentityAdminError {
    IdentityAdminError {
        status: 500,
        detail: Detail::Text(String::from("the caller's role could not be read")),
    }
}
