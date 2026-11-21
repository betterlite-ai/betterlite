use axum::http::StatusCode;
use betterlite_auth::user_roles::UserRole;
use betterlite_logging::callback_config::error::ValidationError;
use serde::Serialize;

use crate::audit_events::record::AuditRecordError;
use crate::member_request_contracts::MemberContractError;

#[derive(Debug, thiserror::Error)]
pub enum TeamCallbackError {
    #[error(transparent)]
    Config { source: ValidationError },
    #[error("{message}")]
    Callback { status: StatusCode, message: String },
    // team_access_denied's body, so a caller who could not manage the team can't probe which team ids exist
    #[error("You do not have access to this team")]
    TeamAccessDenied,
    #[error("the team's callback_settings are not a valid TeamCallbackMetadata")]
    CallbackSettings {
        #[source]
        source: MemberContractError,
    },
    #[error("the team metadata does not convert for decryption")]
    Metadata {
        #[source]
        source: serde_json::Error,
    },
    #[error("the audit snapshot does not serialize")]
    Snapshot {
        #[source]
        source: serde_json::Error,
    },
    #[error("the team-callback audit record is not valid")]
    AuditRecord {
        #[source]
        source: AuditRecordError,
    },
}

// HTTPException's detail: the callback routes send {"error": message}, the access denial a bare string
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ErrorDetail {
    Error { error: String },
    Text(String),
}

// _callback_error's HTTPException: a status and the detail the body carries
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpFailure {
    pub status: StatusCode,
    pub detail: ErrorDetail,
}

impl TeamCallbackError {
    // None is a ValueError or ValidationError LiteLLM raises past the route, which the caller's handler maps
    #[must_use]
    pub fn failure(&self) -> Option<HttpFailure> {
        match self {
            Self::Config { .. } => Some(HttpFailure {
                status: StatusCode::BAD_REQUEST,
                detail: ErrorDetail::Error {
                    error: self.to_string(),
                },
            }),
            Self::Callback { status, .. } => Some(HttpFailure {
                status: *status,
                detail: ErrorDetail::Error {
                    error: self.to_string(),
                },
            }),
            Self::TeamAccessDenied => Some(HttpFailure {
                status: StatusCode::FORBIDDEN,
                detail: ErrorDetail::Text(self.to_string()),
            }),
            Self::CallbackSettings { .. }
            | Self::Metadata { .. }
            | Self::Snapshot { .. }
            | Self::AuditRecord { .. } => None,
        }
    }
}

// _unknown_team_error: only a proxy admin learns that the team id does not exist
#[must_use]
pub fn unknown_team_error(
    team_id: &str,
    role: Option<UserRole>,
    status: StatusCode,
) -> TeamCallbackError {
    if role == Some(UserRole::ProxyAdmin) {
        return TeamCallbackError::Callback {
            status,
            message: format!("Team id = {team_id} does not exist."),
        };
    }
    TeamCallbackError::TeamAccessDenied
}
