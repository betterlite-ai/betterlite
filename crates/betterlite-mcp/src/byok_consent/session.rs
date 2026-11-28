use betterlite_auth::auth_key_lookup::{KeyAuthenticator, key_identity};
use chrono::{DateTime, Utc};
use secrecy::SecretString;

use crate::session_admission::auth::{SessionAuthError, byok_session_auth};

// LiteLLM's _byok_session_auth: without a master key no cookie can verify, so the login is refused.
pub async fn authenticate_session(
    cookie_header: Option<&str>,
    master_key: Option<&SecretString>,
    authenticator: Option<&dyn KeyAuthenticator>,
    now: DateTime<Utc>,
) -> Result<String, SessionAuthError> {
    let Some(secret) = master_key else {
        return Err(SessionAuthError::LoginRequired);
    };
    let auth = byok_session_auth(cookie_header, secret, authenticator, now).await?;
    Ok(key_identity(&auth).user_id.unwrap_or_default())
}
