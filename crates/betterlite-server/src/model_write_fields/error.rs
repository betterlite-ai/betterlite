use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use crate::team_access::error::TeamAccessError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum AdminOnlyField {
    #[strum(to_string = "attach a stored credential (litellm_credential_name)")]
    AttachCredential,
    #[strum(to_string = "detach a stored credential (litellm_credential_name)")]
    DetachCredential,
    #[strum(to_string = "set aws_session_tags")]
    SessionTags,
}

impl AdminOnlyField {
    #[must_use]
    pub const fn param(self) -> &'static str {
        match self {
            Self::AttachCredential | Self::DetachCredential => "litellm_credential_name",
            Self::SessionTags => "aws_session_tags",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModelWriteError {
    #[error("view-only users cannot manage models")]
    ViewOnly,
    #[error("team {team_id} does not exist")]
    TeamMissing { team_id: String },
    #[error("only a proxy admin can act on a model whose team has been deleted")]
    OrphanedModel,
    #[error("the caller is not an admin of team {team_id}; the key's team is {key_team:?}")]
    NotTeamAdmin {
        team_id: String,
        key_team: Option<String>,
    },
    #[error(
        "only a proxy admin, or a team admin naming the team in model_info.team_id, can write models"
    )]
    NotProxyAdmin,
    #[error("a member router write needs a registered auto-router policy and model catalog")]
    MemberRouterUnavailable,
    #[error("only a proxy admin can {field} on a model")]
    AdminOnly { field: AdminOnlyField },
    #[error("the model's team could not be read")]
    TeamRead(#[source] Box<TeamQueryError>),
    #[error("the caller's team role could not be read")]
    TeamAccess(#[source] Box<TeamAccessError>),
}

// LiteLLM raises an HTTPException whose detail is text or {"error": ...}, or a ProxyException with type, param and code
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detail {
    Message(String),
    Error(String),
    Proxy(Box<ProxyException>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub status: StatusCode,
    pub detail: Detail,
}

impl ModelWriteError {
    // None is a failed read LiteLLM lets escape as a server error
    #[must_use]
    pub fn rejection(&self) -> Option<Rejection> {
        let message = self.to_string();
        let (status, detail) = match self {
            Self::ViewOnly => (StatusCode::FORBIDDEN, Detail::Message(message)),
            Self::MemberRouterUnavailable => (StatusCode::BAD_REQUEST, Detail::Message(message)),
            Self::TeamMissing { .. } => (StatusCode::BAD_REQUEST, Detail::Error(message)),
            Self::OrphanedModel | Self::NotTeamAdmin { .. } | Self::NotProxyAdmin => {
                (StatusCode::FORBIDDEN, Detail::Error(message))
            }
            Self::AdminOnly { field } => {
                let exception = Box::new(ProxyException::new(ProxyExceptionInit {
                    message,
                    error_type: ProxyErrorTypes::AuthError.to_string(),
                    param: Some(field.param().to_owned()),
                    code: ExceptionCode::Status(403_i64),
                    headers: BTreeMap::new(),
                    openai_code: None,
                    provider_specific_fields: BTreeMap::new(),
                }));
                (StatusCode::FORBIDDEN, Detail::Proxy(exception))
            }
            Self::TeamRead(_) | Self::TeamAccess(_) => return None,
        };
        Some(Rejection { status, detail })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MemberCallError<Refusal> {
    #[error(transparent)]
    Access(#[from] ModelWriteError),
    #[error("the member router policy refused the write")]
    Refused(#[source] Refusal),
}
