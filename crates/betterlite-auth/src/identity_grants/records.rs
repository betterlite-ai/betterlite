use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;

use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_user_lookup::error::UserLookupError;
use crate::membership_load::error::MembershipLoadError;

// The user a credential names, plus the hints get_user_object falls back to when the id matches no row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserQuery {
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub sso_user_id: Option<String>,
    pub upsert: bool,
}

// resolve_identity's answer: the team is the JWT builder's choice, so it is not read here
#[derive(Clone, Debug)]
pub struct ResolvedIdentity {
    pub user: Option<LiteLLMUserTable>,
    pub membership: Option<LiteLLMTeamMembership>,
    pub effective_user_id: Option<String>,
}

// effective_user_id is the row's id when an SSO or email match found a legacy row under another id
#[derive(Clone, Debug)]
pub struct ResolvedGrants {
    pub identity: ResolvedIdentity,
    pub team: Option<LiteLLMTeamTableCachedObj>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantDenial {
    UserGone { user_id: String },
    TeamGone { team_id: String },
    NotAMember { user_id: String, team_id: String },
}

// What the user and membership reads raise, left uncaught by resolve_identity.
#[derive(Debug, thiserror::Error)]
pub enum IdentityLoadError {
    #[error(transparent)]
    User(Box<UserLookupError>),
    #[error(transparent)]
    Membership(Box<MembershipLoadError>),
}

impl IdentityLoadError {
    // UserNotFoundError proves the row is gone; every other failure says nothing about the caller
    #[must_use]
    pub fn proves_user_gone(&self) -> bool {
        match self {
            Self::User(failure) => match failure.as_ref() {
                UserLookupError::NotFound(_) => true,
                UserLookupError::NoDatabase
                | UserLookupError::Unavailable(_)
                | UserLookupError::UserMissing(_)
                | UserLookupError::PreLookup(_) => false,
            },
            Self::Membership(_) => false,
        }
    }
}

// LookupDegraded: the reader's own failure, kept so the caller can tell it from a proved absence
#[derive(Debug, thiserror::Error)]
pub enum DegradedRead {
    #[error(transparent)]
    Identity(IdentityLoadError),
    #[error(transparent)]
    Team(Box<TeamLookupError>),
}

#[derive(Debug)]
pub enum GrantOutcome {
    Resolved(Box<ResolvedGrants>),
    Denied(GrantDenial),
    Degraded(DegradedRead),
}
