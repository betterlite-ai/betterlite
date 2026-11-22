use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::store::StoredPollState;

// the trusted identity B080 records at admission; an empty field means it was absent
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollOwner {
    pub user_id: String,
    pub team_id: String,
    pub key_fingerprint: String,
}

impl PollOwner {
    #[must_use]
    pub fn from_principal(principal: &Principal, key_fingerprint: Option<&str>) -> Self {
        Self {
            user_id: principal
                .user
                .as_ref()
                .map(|user| user.id.clone())
                .unwrap_or_default(),
            team_id: principal
                .teams
                .first()
                .map(|team| team.id.clone())
                .unwrap_or_default(),
            key_fingerprint: key_fingerprint.unwrap_or_default().to_owned(),
        }
    }
}

// the caller B929's policy reads: the recorded identity plus the proxy-admin role
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PollCaller {
    pub identity: PollOwner,
    pub platform_admin: bool,
}

impl PollCaller {
    #[must_use]
    pub fn from_principal(principal: &Principal, key_fingerprint: Option<&str>) -> Self {
        Self {
            identity: PollOwner::from_principal(principal, key_fingerprint),
            platform_admin: principal.roles.contains(&Role::PlatformAdmin),
        }
    }
}

// AuthDisabled is the configured auth-disabled mode, where the route has no trusted principal at all
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollAccess<'a> {
    AuthDisabled,
    Caller(&'a PollCaller),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PollDenial {
    #[error("the polling response belongs to a different user")]
    User,
    #[error("the polling response belongs to a different team")]
    Team,
    #[error("the polling response belongs to a different key")]
    KeyFingerprint,
    #[error("the polling response has no owner")]
    OwnerlessRecord,
    #[error("the polling response is anonymous")]
    AnonymousRecord,
}

// B929's order: proxy admin, then the recorded user, then the recorded team
pub fn authorize(state: &StoredPollState, access: PollAccess<'_>) -> Result<(), PollDenial> {
    let PollAccess::Caller(caller) = access else {
        return Ok(());
    };
    if caller.platform_admin {
        return Ok(());
    }
    match state {
        StoredPollState::Legacy(_) => Err(PollDenial::OwnerlessRecord),
        StoredPollState::Record(record) => match record.owner.as_ref() {
            None => Err(PollDenial::AnonymousRecord),
            Some(owner) => check_owner(owner, caller),
        },
    }
}

fn check_owner(owner: &PollOwner, caller: &PollCaller) -> Result<(), PollDenial> {
    let identity = &caller.identity;
    if identity.user_id.is_empty() && identity.team_id.is_empty() {
        return (identity.key_fingerprint == owner.key_fingerprint)
            .then_some(())
            .ok_or(PollDenial::KeyFingerprint);
    }
    if !owner.user_id.is_empty() && identity.user_id != owner.user_id {
        return Err(PollDenial::User);
    }
    if !owner.team_id.is_empty() && identity.team_id != owner.team_id {
        return Err(PollDenial::Team);
    }
    Ok(())
}
