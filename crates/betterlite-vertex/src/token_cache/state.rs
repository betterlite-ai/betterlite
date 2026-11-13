use std::time::{Duration, SystemTime};

use super::record::CredentialSnapshot;

// google-auth treats a token as stale inside the last 3:45 before its expiry
pub const REFRESH_THRESHOLD: Duration = Duration::from_secs(225);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenState {
    Fresh,
    Stale,
    Invalid,
}

// LiteLLM's get_access_token refreshes on credentials.expired, which google-auth derives from the expiry alone
#[must_use]
pub fn needs_refresh(credential: &CredentialSnapshot, now: SystemTime) -> bool {
    let expiry_passed = credential.expires_at.is_some_and(|expires_at| {
        expires_at
            .duration_since(now)
            .map_or(true, |remaining| remaining <= REFRESH_THRESHOLD)
    });
    expiry_passed
        || matches!(
            credential.state,
            Some(TokenState::Stale | TokenState::Invalid)
        )
        || credential.expired == Some(true)
}

// LiteLLM's _get_token_state: the reported state, else google-auth's expiry projection, else the expired/valid fallback
#[must_use]
pub fn token_state(credential: &CredentialSnapshot, now: SystemTime) -> TokenState {
    if let Some(state) = credential.state {
        return state;
    }
    if let Some(expires_at) = credential.expires_at {
        if credential.token.is_none() {
            return TokenState::Invalid;
        }
        return match expires_at.duration_since(now) {
            Ok(remaining) if remaining > REFRESH_THRESHOLD => TokenState::Fresh,
            Ok(remaining) if !remaining.is_zero() => TokenState::Stale,
            _ => TokenState::Invalid,
        };
    }
    if credential.expired.unwrap_or(true) {
        return TokenState::Invalid;
    }
    if credential.valid.unwrap_or(false) {
        return TokenState::Fresh;
    }
    TokenState::Invalid
}
