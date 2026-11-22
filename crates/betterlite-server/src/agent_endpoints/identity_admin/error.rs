use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::agent_identity::error::AgentIdentityError;
use serde::Serialize;

// LiteLLM's HTTPException detail: a plain message, or the object a few checks answer with
#[derive(Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum Detail {
    Text(String),
    Object { error: String },
}

#[derive(Serialize)]
struct DetailBody {
    detail: Detail,
}

#[derive(Debug)]
pub(crate) struct IdentityAdminError {
    pub(crate) status: u16,
    pub(crate) detail: Detail,
}

impl From<AgentIdentityError> for IdentityAdminError {
    fn from(error: AgentIdentityError) -> Self {
        Self {
            status: error.status,
            detail: Detail::Text(error.message),
        }
    }
}

impl IntoResponse for IdentityAdminError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (
            status,
            Json(DetailBody {
                detail: self.detail,
            }),
        )
            .into_response()
    }
}
