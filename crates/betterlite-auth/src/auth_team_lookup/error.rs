use betterlite_db::team_rows::decoder::TeamRowError;

use super::ports::{TeamCacheError, TeamStoreError};
use crate::auth_prefetch::permission::lookup::PermissionError;
use crate::auth_user_lookup::ports::PortError;
use crate::resolved_key::auth::AuthError as CredentialError;

// What leaves each lookup. Every variant with a status is LiteLLM's HTTPException; B757 renders it.
#[derive(Debug, thiserror::Error)]
pub enum TeamLookupError {
    #[error("No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys")]
    NoDatabase,
    // the cache read sits before the source's try block, so its failure is not relabeled
    #[error(transparent)]
    CacheRead(Box<TeamCacheError>),
    #[error("Team doesn't exist in cache + check_cache_only=True. Team={team_id}.")]
    NotInCache { team_id: String },
    // TeamNotFoundError: the database answered and the row is absent, unlike TeamUnavailable
    #[error("Team doesn't exist in db. Team={team_id}. Create team via `/team/new` call.")]
    TeamNotFound { team_id: String },
    #[error("Team doesn't exist in db. Team={team_id}. Create team via `/team/new` call.")]
    TeamUnavailable {
        team_id: String,
        #[source]
        cause: Box<TeamFailure>,
    },
    #[error(
        "Team with alias '{team_alias}' doesn't exist in db. Create team via `/team/new` call."
    )]
    AliasNotFound { team_alias: String },
    #[error(
        "Multiple teams found with alias '{team_alias}'. Please use team_id_jwt_field instead or ensure team aliases are unique."
    )]
    AliasAmbiguous { team_alias: String },
    #[error("Error looking up team by alias '{team_alias}': {cause}")]
    AliasFailure {
        team_alias: String,
        #[source]
        cause: Box<TeamFailure>,
    },
    #[error("Access group doesn't exist in db. Access group={access_group_id}.")]
    AccessGroupNotFound { access_group_id: String },
    #[error("Access group doesn't exist in db. Access group={access_group_id}. Error: {cause}")]
    AccessGroupFailure {
        access_group_id: String,
        #[source]
        cause: Box<TeamFailure>,
    },
    #[error("Access group policy is unavailable")]
    AccessPolicyUnavailable {
        access_group_id: String,
        #[source]
        cause: Box<TeamFailure>,
    },
}

// LiteLLM's detail: {"error": message} for every HTTPException here except the writer's 503 string
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    Error,
    Text,
}

impl TeamLookupError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::NoDatabase | Self::CacheRead(_) => None,
            Self::AliasAmbiguous { .. } => Some(400),
            Self::AliasFailure { .. } => Some(500),
            Self::AccessPolicyUnavailable { .. } => Some(503),
            Self::NotInCache { .. }
            | Self::TeamNotFound { .. }
            | Self::TeamUnavailable { .. }
            | Self::AliasNotFound { .. }
            | Self::AccessGroupNotFound { .. }
            | Self::AccessGroupFailure { .. } => Some(404),
        }
    }

    #[must_use]
    pub const fn detail(&self) -> Option<Detail> {
        match self {
            Self::NoDatabase | Self::CacheRead(_) => None,
            Self::AccessPolicyUnavailable { .. } => Some(Detail::Text),
            Self::NotInCache { .. }
            | Self::TeamNotFound { .. }
            | Self::TeamUnavailable { .. }
            | Self::AliasNotFound { .. }
            | Self::AliasAmbiguous { .. }
            | Self::AliasFailure { .. }
            | Self::AccessGroupNotFound { .. }
            | Self::AccessGroupFailure { .. } => Some(Detail::Error),
        }
    }
}

// The failures inside the source's try blocks, kept as the cause of the HTTP error they become.
#[derive(Debug, thiserror::Error)]
pub enum TeamFailure {
    #[error("the team was read moments ago and was absent")]
    Throttled,
    #[error(transparent)]
    Store(Box<TeamStoreError>),
    #[error("the team row did not validate")]
    Row(#[source] Box<TeamRowError>),
    #[error("the access group row did not validate")]
    AccessRow(#[source] serde_json::Error),
    #[error(transparent)]
    Permission(Box<PermissionError>),
    #[error(transparent)]
    Cache(Box<TeamCacheError>),
    #[error("team_id_upsert needs a team provisioner and none is installed")]
    NoProvisioner,
    #[error(transparent)]
    Provision(Box<PortError>),
    #[error(transparent)]
    Principal(Box<CredentialError>),
    #[error("the team could not be recorded as read")]
    Cadence(#[source] serde_json::Error),
}
