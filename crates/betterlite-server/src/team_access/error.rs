use std::collections::BTreeMap;

use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

#[derive(Debug, thiserror::Error)]
pub enum TeamAccessError {
    // access.py team_access_denied: an HTTPException(403) with this detail and no type or param
    #[error("You do not have access to this team")]
    Denied,
    #[error(
        "the caller is not a member of team {team_id}, so it can't call {route} for that team's keys"
    )]
    NotTeamMember { team_id: String, route: String },
    #[error("members of team {team_id} may call only {allowed:?}, not {route}")]
    MemberRoute {
        team_id: String,
        route: String,
        allowed: Vec<String>,
    },
    #[error(
        "a non-admin caller can't assign access groups to a personal key; denied groups: {denied:?}"
    )]
    PersonalKeyGroups { denied: Vec<String> },
    #[error("team {team_id} hasn't enabled the '/key/access_group_assignment' member permission")]
    GroupOptIn { team_id: String },
    #[error("'{value}' is not a valid KeyManagementRoutes")]
    UnknownPermission { value: String },
    #[error("the caller's organization role could not be read")]
    OrgRoles(#[source] Box<UserLookupError>),
    #[error("the key's team could not be read")]
    TeamLookup(#[source] Box<TeamLookupError>),
    #[error("the caller's identity could not be read")]
    Caller(#[source] serde_json::Error),
}

impl TeamAccessError {
    // the HTTP status LiteLLM raises; None is an exception the caller's handler maps
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Denied | Self::PersonalKeyGroups { .. } | Self::GroupOptIn { .. } => {
                Some(403_u16)
            }
            Self::NotTeamMember { .. } | Self::MemberRoute { .. } => Some(401_u16),
            Self::UnknownPermission { .. }
            | Self::OrgRoles(_)
            | Self::TeamLookup(_)
            | Self::Caller(_) => None,
        }
    }

    // team_member_permission_checks.py raises ProxyException(type=team_member_permission_error, param=route, code=401)
    #[must_use]
    pub fn proxy_exception(&self) -> Option<ProxyException> {
        let (Self::NotTeamMember { route, .. } | Self::MemberRoute { route, .. }) = self else {
            return None;
        };
        Some(ProxyException::new(ProxyExceptionInit {
            message: self.to_string(),
            error_type: ProxyErrorTypes::TeamMemberPermissionError.to_string(),
            param: Some(route.clone()),
            code: ExceptionCode::Status(401_i64),
            headers: BTreeMap::new(),
            openai_code: None,
            provider_specific_fields: BTreeMap::new(),
        }))
    }
}
