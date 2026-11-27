mod tests;

use secrecy::SecretString;
use uuid::Uuid;

use super::ConsentError;
use super::location::redirect_location;
use super::store::{AUTH_CODE_TTL_SECONDS, AuthCodeStore};
use crate::byok_token_exchange::ports::{AuthCodeLookup as _, AuthCodeRecord};
use crate::redirect_trust::loopback::validate_loopback_redirect_uri;

pub const REQUIRED_CHALLENGE_METHOD: &str = "S256";

pub struct PostForm {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
    pub server_id: String,
    pub api_key: SecretString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub code: String,
    pub location: String,
}

// LiteLLM's byok_authorize_post, in its order: purge, validate, capacity, method, identity, store.
pub fn authorize_post(
    form: &PostForm,
    user_id: &str,
    codes: &AuthCodeStore,
    now: f64,
) -> Result<Accepted, ConsentError> {
    codes.purge_expired(now);
    validate_loopback_redirect_uri(&form.redirect_uri)?;
    codes.admit()?;
    if form.code_challenge_method != REQUIRED_CHALLENGE_METHOD {
        return Err(ConsentError::UnsupportedChallengeMethod);
    }
    if user_id.is_empty() {
        return Err(ConsentError::LoginRequired);
    }
    let code = Uuid::new_v4().to_string();
    let record = AuthCodeRecord {
        api_key: form.api_key.clone(),
        server_id: form.server_id.clone(),
        code_challenge: form.code_challenge.clone(),
        redirect_uri: form.redirect_uri.clone(),
        client_id: form.client_id.clone(),
        user_id: user_id.to_owned(),
        expires_at: now + f64::from(AUTH_CODE_TTL_SECONDS),
    };
    codes.insert(code.clone(), record);
    let location = redirect_location(&form.redirect_uri, &code, &form.state);
    Ok(Accepted { code, location })
}
