use chrono::{DateTime, Utc};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::Expiry;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_models::savings_evidence::billed_at::decode_billed_at;

use crate::resolved_key::auth::UserAPIKeyAuth;

mod tests;

// LiteLLM reads blocked with "is True", and a malformed expiry string is inactive rather than an error
#[must_use]
pub fn key_is_active(key: &UserAPIKeyAuth, now: DateTime<Utc>) -> bool {
    if key.view.token.blocked.is_some_and(|blocked| blocked.0) {
        return false;
    }
    let Some(expires) = key.view.token.expires.as_ref() else {
        return true;
    };
    match expires {
        Expiry::DateTime(value) => *value >= now,
        Expiry::Text(text) => decode_billed_at(text).is_some_and(|instant| instant >= now),
    }
}

#[must_use]
pub fn active_key_user_id(key: &UserAPIKeyAuth, now: DateTime<Utc>) -> Option<&str> {
    if !key_is_active(key, now) {
        return None;
    }
    key.view.token.user_id.as_deref()
}

// Python's "is False" leaves null, 0 and "false" active, so only a literal JSON false deactivates
#[must_use]
pub fn user_scim_deactivated(user: &LiteLLMUserTable) -> bool {
    let Some(metadata) = user.metadata.as_ref() else {
        return false;
    };
    metadata.0.get("scim_active") == Some(&JsonData::Bool(false))
}
