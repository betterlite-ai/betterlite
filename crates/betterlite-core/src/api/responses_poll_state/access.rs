use thiserror::Error;

use super::decode::StoredPollState;
use super::owner::{PollCaller, PollOwner};

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
    if owner.user_id.is_empty() && owner.team_id.is_empty() {
        return if caller.key_fingerprint.as_deref().unwrap_or_default() == owner.key_fingerprint {
            Ok(())
        } else {
            Err(PollDenial::KeyFingerprint)
        };
    }
    if !owner.user_id.is_empty() && caller.user_id.as_deref() != Some(owner.user_id.as_str()) {
        return Err(PollDenial::User);
    }
    if !owner.team_id.is_empty() && caller.team_id.as_deref() != Some(owner.team_id.as_str()) {
        return Err(PollDenial::Team);
    }
    Ok(())
}
