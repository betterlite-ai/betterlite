use std::time::Duration;

use chrono::{NaiveDateTime, TimeDelta};

use crate::database_tokens::expiry::{ExpiryError, TokenAuthKind, parse_token_expiration};

use super::extraction::extract_refresh_password;

pub const TOKEN_REFRESH_BUFFER_SECONDS: i64 = 180;
pub const FALLBACK_REFRESH_SECONDS: u64 = 600;
pub const MIN_REFRESH_SLEEP_SECONDS: u64 = 30;

const TOKEN_REFRESH_BUFFER: TimeDelta = TimeDelta::seconds(TOKEN_REFRESH_BUFFER_SECONDS);

pub struct RefreshSchedule {
    pub seconds: f64,
    pub wait: Duration,
}

pub struct RefreshQuery<'url> {
    pub kind: Option<TokenAuthKind>,
    pub token_url: Option<&'url str>,
    pub now: NaiveDateTime,
}

pub fn parse_expiration(
    kind: Option<TokenAuthKind>,
    token: Option<&str>,
) -> Result<Option<NaiveDateTime>, ExpiryError> {
    let (Some(kind_value), Some(token_value)) = (kind, token) else {
        return Ok(None);
    };
    parse_token_expiration(kind_value, token_value)
}

// A missing URL and an undecodable token both mean no expiration; each caller picks its own fallback.
fn expiration_of(
    kind: Option<TokenAuthKind>,
    token_url: Option<&str>,
) -> Result<Option<NaiveDateTime>, ExpiryError> {
    let Some(url) = token_url else {
        return Ok(None);
    };
    let token = extract_refresh_password(url);
    parse_expiration(kind, token.as_deref())
}

pub fn seconds_until_refresh(query: &RefreshQuery<'_>) -> Result<RefreshSchedule, ExpiryError> {
    let Some(expiration) = expiration_of(query.kind, query.token_url)? else {
        tracing::debug!(
            "Could not parse token expiration, using fallback interval of {FALLBACK_REFRESH_SECONDS}s"
        );
        let wait = Duration::from_secs(FALLBACK_REFRESH_SECONDS);
        return Ok(RefreshSchedule {
            seconds: wait.as_secs_f64(),
            wait,
        });
    };
    let refresh_at = expiration
        .checked_sub_signed(TOKEN_REFRESH_BUFFER)
        .ok_or(ExpiryError::OutOfRange)?;
    let wait = refresh_at
        .signed_duration_since(query.now)
        .to_std()
        .unwrap_or(Duration::ZERO)
        .max(Duration::from_secs(MIN_REFRESH_SLEEP_SECONDS));
    Ok(RefreshSchedule {
        seconds: wait.as_secs_f64(),
        wait,
    })
}

pub fn is_token_expired(query: &RefreshQuery<'_>) -> Result<bool, ExpiryError> {
    let Some(url) = query.token_url else {
        return Ok(true);
    };
    let Some(expiration) = expiration_of(query.kind, Some(url))? else {
        tracing::debug!("Could not parse token expiration, treating as expired");
        return Ok(true);
    };
    Ok(query.now > expiration)
}

pub fn token_refresh_not_needed(query: &RefreshQuery<'_>) -> Result<bool, ExpiryError> {
    let Some(expiration) = expiration_of(query.kind, query.token_url)? else {
        return Ok(false);
    };
    Ok(expiration.signed_duration_since(query.now) > TOKEN_REFRESH_BUFFER)
}
