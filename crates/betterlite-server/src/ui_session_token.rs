mod tests;

use betterlite_auth::sso_types::identity::ReturnedUiTokenObject;
use betterlite_config::sections::key_access::KeyGeneration;
use betterlite_config::sections::ui_session_token::Settings;
use betterlite_core::duration::error::DurationError;
use betterlite_core::duration::seconds::duration_in_seconds;
use chrono::{DateTime, Duration, Utc};
use jiff::tz::TimeZoneDatabase;
use jsonwebtoken::errors::Error as JwtError;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UiSessionTokenError {
    // boxed so the enum's variants stay within rustc's variant-size-difference lint
    #[error("the UI session duration is invalid")]
    Duration(#[source] Box<DurationError>),
    #[error("the UI session duration does not fit the login time")]
    Overflow,
    #[error("the master key is empty, so the UI session token cannot be signed")]
    EmptyMasterKey,
    #[error("the UI session token could not be signed")]
    Encode(#[source] JwtError),
}

// every field of the token object in its serde order, then the expiry LiteLLM appends to the claims
#[derive(Serialize)]
struct UiSessionClaims<'token> {
    #[serde(flatten)]
    token: &'token ReturnedUiTokenObject,
    exp: i64,
}

pub fn ui_session_exp_timestamp(
    settings: &Settings,
    now: DateTime<Utc>,
    database: &TimeZoneDatabase,
) -> Result<i64, UiSessionTokenError> {
    let parsed = duration_in_seconds(&settings.duration, now, None, database)
        .map_err(|error| UiSessionTokenError::Duration(Box::new(error)))?;
    let offset = Duration::try_seconds(parsed.seconds).ok_or(UiSessionTokenError::Overflow)?;
    let expiry = now
        .checked_add_signed(offset)
        .ok_or(UiSessionTokenError::Overflow)?;
    // Python's int() cuts the fractional second instead of rounding it
    Ok(expiry.timestamp())
}

pub fn encode_ui_session_jwt(
    token: &ReturnedUiTokenObject,
    master_key: &SecretString,
    settings: &Settings,
    now: DateTime<Utc>,
    database: &TimeZoneDatabase,
) -> Result<String, UiSessionTokenError> {
    // LiteLLM builds the claims, and with them the exp, before PyJWT looks at the key
    let exp = ui_session_exp_timestamp(settings, now, database)?;
    if master_key.expose_secret().is_empty() {
        return Err(UiSessionTokenError::EmptyMasterKey);
    }
    let claims = UiSessionClaims { token, exp };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(master_key.expose_secret().as_bytes()),
    )
    .map_err(UiSessionTokenError::Encode)
}

#[must_use]
// ast-grep-ignore: no-long-names - LiteLLM's name, called by the login and settings routes
pub fn disabled_non_admin_personal_key_creation(key_generation: &KeyGeneration) -> bool {
    if !key_generation.configured {
        return false;
    }
    let Some(personal) = &key_generation.personal else {
        return false;
    };
    personal
        .allowed_user_roles
        .as_ref()
        .is_some_and(|roles| roles.iter().any(|role| role == "proxy_admin"))
}
