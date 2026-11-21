use std::collections::BTreeMap;

use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use super::records::GrantDenial;
use crate::auth_team_lookup::error::{Detail, TeamLookupError};

// raise_public's three exceptions; B757 renders the HTTP ones as auth_error with param "None"
#[derive(Debug, thiserror::Error)]
pub enum PublicDenial {
    #[error(transparent)]
    UserGone(Box<ProxyException>),
    // B737's TeamNotFoundError, always its TeamNotFound variant
    #[error(transparent)]
    TeamGone(TeamLookupError),
    #[error("Team '{team_id}' is not in your team memberships.")]
    NotAMember { team_id: String },
}

impl PublicDenial {
    // the HTTPException's status_code and detail shape; None is a ProxyException that passes through as raised
    #[must_use]
    pub const fn http(&self) -> Option<(u16, Detail)> {
        match self {
            Self::UserGone(_) => None,
            Self::TeamGone(error) => error.http(),
            Self::NotAMember { .. } => Some((403, Detail::Text)),
        }
    }
}

#[must_use]
pub fn raise_public(denial: GrantDenial) -> PublicDenial {
    match denial {
        GrantDenial::UserGone { user_id } => {
            PublicDenial::UserGone(Box::new(ProxyException::new(ProxyExceptionInit {
                message: format!("Authentication Error, user '{user_id}' no longer exists."),
                error_type: ProxyErrorTypes::AuthError.to_string(),
                param: Some("user_id".to_owned()),
                code: ExceptionCode::Status(401),
                headers: BTreeMap::new(),
                openai_code: None,
                provider_specific_fields: BTreeMap::new(),
            })))
        }
        GrantDenial::TeamGone { team_id } => {
            PublicDenial::TeamGone(TeamLookupError::TeamNotFound { team_id })
        }
        GrantDenial::NotAMember { team_id, .. } => PublicDenial::NotAMember { team_id },
    }
}
