use std::collections::BTreeMap;

use betterlite_auth::auth_key_lookup::KeyAuthenticator;
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};
use http::StatusCode;
use thiserror::Error;

use super::identity::session_identity_from_cookie;
use super::liveness::session_key_is_live;

// FastAPI writes the dependency's refusal as ``{"detail": "login_required"}`` with this status.
pub const LOGIN_REQUIRED: &str = "login_required";
const BYOK_SESSION_API_KEY: &str = "byok_session_cookie";

#[derive(Debug, Error)]
pub enum SessionAuthError {
    #[error("the dashboard session cookie did not admit the request")]
    LoginRequired,
    #[error("the dashboard session principal did not construct")]
    Construct(#[source] AuthError),
}

impl SessionAuthError {
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::LoginRequired => StatusCode::UNAUTHORIZED,
            Self::Construct(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

// LiteLLM's _byok_session_auth: a live UI session cookie, then the UserAPIKeyAuth the route receives.
pub async fn byok_session_auth(
    cookie_header: Option<&str>,
    master_key: &str,
    authenticator: Option<&dyn KeyAuthenticator>,
    now: DateTime<Utc>,
) -> Result<UserAPIKeyAuth, SessionAuthError> {
    let Some(identity) = session_identity_from_cookie(cookie_header, master_key, now) else {
        return Err(SessionAuthError::LoginRequired);
    };
    if !session_key_is_live(authenticator, identity.session_key.as_deref()).await {
        return Err(SessionAuthError::LoginRequired);
    }
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    fields.insert(
        "api_key".to_owned(),
        JsonData::String(BYOK_SESSION_API_KEY.to_owned()),
    );
    fields.insert("user_id".to_owned(), JsonData::String(identity.user_id));
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).map_err(SessionAuthError::Construct)
}
