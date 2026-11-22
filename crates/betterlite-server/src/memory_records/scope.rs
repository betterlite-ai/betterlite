use axum::http::StatusCode;
use serde::Serialize;

use super::caller::{MemoryCaller, is_admin};
use super::error::MemoryHttpError;

// the (user_id, team_id) a new row is stamped with
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Scope {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
}

// memory_endpoints.py _resolve_scope: admins may override either dimension, everyone else their own
pub fn resolve_scope(
    caller: &MemoryCaller,
    requested_user_id: Option<&str>,
    requested_team_id: Option<&str>,
) -> Result<Scope, MemoryHttpError> {
    if is_admin(caller) {
        return Ok(Scope {
            user_id: requested_user_id
                .or(caller.user_id.as_deref())
                .map(str::to_owned),
            team_id: requested_team_id
                .or(caller.team_id.as_deref())
                .map(str::to_owned),
        });
    }
    let user_mismatch =
        requested_user_id.is_some_and(|requested| Some(requested) != caller.user_id.as_deref());
    if user_mismatch {
        return Err(MemoryHttpError {
            status: StatusCode::FORBIDDEN,
            detail: "Only proxy admins may set user_id to a different user.".to_owned(),
        });
    }
    let team_mismatch =
        requested_team_id.is_some_and(|requested| Some(requested) != caller.team_id.as_deref());
    if team_mismatch {
        return Err(MemoryHttpError {
            status: StatusCode::FORBIDDEN,
            detail: "Only proxy admins may set team_id to a different team.".to_owned(),
        });
    }
    if caller.user_id.is_none() && caller.team_id.is_none() {
        return Err(MemoryHttpError {
            status: StatusCode::BAD_REQUEST,
            detail: "Cannot create a memory entry without a user_id or team_id. Authenticate with a key that has a user_id or team_id, or call as a proxy admin.".to_owned(),
        });
    }
    Ok(Scope {
        user_id: caller.user_id.clone(),
        team_id: caller.team_id.clone(),
    })
}
