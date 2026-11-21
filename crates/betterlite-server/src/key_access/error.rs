use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_logging::callback_config::error::PayloadError;

use crate::key_fields::error::DecodeError;
use crate::team_access::error::TeamAccessError;

// how LiteLLM's HTTPException carries the message: the detail itself, or {"error": message}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DetailShape {
    Text,
    Error,
}

#[derive(Debug, thiserror::Error)]
pub enum KeyAccessError {
    #[error("custom key values are turned off on this proxy, so keys are always generated")]
    CustomKeyDisabled,
    #[error("a caller can only create or update its own keys, not keys for user {user_id}")]
    NotOwner { user_id: String },
    #[error("user {user_id} is not a member of team {team_id}")]
    AssignedNotMember { user_id: String, team_id: String },
    #[error("the caller is not a member of team {team_id}")]
    CallerNotMember { team_id: String },
    #[error("team role {role} may not create team keys; allowed roles: {allowed:?}")]
    RoleNotAllowed { role: String, allowed: Vec<String> },
    #[error("the key request must set {param}")]
    MissingParam { param: String },
    #[error("personal keys are limited to the roles {allowed:?}; the caller's role is {role:?}")]
    PersonalRole {
        role: Option<String>,
        allowed: Vec<String>,
    },
    #[error("a service account can only create keys for its own team {team_id:?}")]
    ServiceAccountTeam { team_id: Option<String> },
    #[error("team {team_id} was not found in the database")]
    TeamNotFound { team_id: String },
    #[error("a service account key needs a team_id in the request body")]
    ServiceTeamRequired,
    #[error("service account keys need a connected database")]
    ServiceDatabaseRequired,
    #[error("team {team_id} does not exist, so it can't own a service account key")]
    ServiceTeamUnknown { team_id: String },
    #[error("{0}")]
    LoggingMetadata(#[source] Box<PayloadError>),
    #[error("the key's metadata could not be read")]
    Metadata(#[source] serde_json::Error),
    #[error(transparent)]
    Team(Box<TeamAccessError>),
    #[error("the key request could not be read")]
    Request(#[source] Box<DecodeError>),
    #[error("the team table could not be read")]
    TeamRead(#[source] Box<TeamQueryError>),
}

impl KeyAccessError {
    // the HTTP status LiteLLM raises; None is an exception the route's handler maps
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::CustomKeyDisabled | Self::NotOwner { .. } | Self::ServiceAccountTeam { .. } => {
                Some(403_u16)
            }
            Self::AssignedNotMember { .. }
            | Self::CallerNotMember { .. }
            | Self::RoleNotAllowed { .. }
            | Self::MissingParam { .. }
            | Self::PersonalRole { .. }
            | Self::TeamNotFound { .. }
            | Self::ServiceTeamRequired
            | Self::ServiceDatabaseRequired
            | Self::ServiceTeamUnknown { .. }
            | Self::LoggingMetadata(_) => Some(400_u16),
            Self::Team(team) => team.status(),
            Self::Metadata(_) | Self::Request(_) | Self::TeamRead(_) => None,
        }
    }

    #[must_use]
    pub fn detail(&self) -> Option<DetailShape> {
        match self {
            Self::CustomKeyDisabled | Self::LoggingMetadata(_) => Some(DetailShape::Error),
            Self::NotOwner { .. }
            | Self::AssignedNotMember { .. }
            | Self::CallerNotMember { .. }
            | Self::RoleNotAllowed { .. }
            | Self::MissingParam { .. }
            | Self::PersonalRole { .. }
            | Self::ServiceAccountTeam { .. }
            | Self::TeamNotFound { .. }
            | Self::ServiceTeamRequired
            | Self::ServiceDatabaseRequired
            | Self::ServiceTeamUnknown { .. } => Some(DetailShape::Text),
            Self::Team(team) => match **team {
                TeamAccessError::Denied
                | TeamAccessError::PersonalKeyGroups { .. }
                | TeamAccessError::GroupOptIn { .. } => Some(DetailShape::Text),
                TeamAccessError::NotTeamMember { .. }
                | TeamAccessError::MemberRoute { .. }
                | TeamAccessError::UnknownPermission { .. }
                | TeamAccessError::OrgRoles(_)
                | TeamAccessError::TeamLookup(_)
                | TeamAccessError::Caller(_) => None,
            },
            Self::Metadata(_) | Self::Request(_) | Self::TeamRead(_) => None,
        }
    }

    // a denied team member keeps B800's ProxyException(team_member_permission_error, route, 401)
    #[must_use]
    pub fn proxy_exception(&self) -> Option<ProxyException> {
        let Self::Team(team) = self else {
            return None;
        };
        team.proxy_exception()
    }
}

impl From<TeamAccessError> for KeyAccessError {
    fn from(error: TeamAccessError) -> Self {
        Self::Team(Box::new(error))
    }
}

impl From<DecodeError> for KeyAccessError {
    fn from(error: DecodeError) -> Self {
        Self::Request(Box::new(error))
    }
}
