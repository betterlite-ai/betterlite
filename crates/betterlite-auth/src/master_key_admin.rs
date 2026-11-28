use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::key_digest::hashing::hash_token;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use crate::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use crate::user_roles::UserRole;

mod tests;

// LiteLLM's LITELLM_PROXY_MASTER_KEY_ALIAS, the master key's stand-in so the real key never reaches logs or spend rows
pub const MASTER_KEY_ALIAS: &str = "litellm_proxy_master_key";

#[derive(Debug, thiserror::Error)]
pub enum MasterKeyAdminError {
    #[error(transparent)]
    Auth(#[from] AuthError),
}

pub fn master_key_admin_auth(
    key_hash: &str,
    master_key: Option<&SecretString>,
    admin_name: &str,
) -> Result<Option<UserAPIKeyAuth>, MasterKeyAdminError> {
    let Some(master_secret) = master_key else {
        return Ok(None);
    };
    let master_text = master_secret.expose_secret();
    if master_text.is_empty() {
        return Ok(None);
    }
    let master_digest = hash_token(master_text);
    if !bool::from(key_hash.as_bytes().ct_eq(master_digest.as_bytes())) {
        return Ok(None);
    }
    let fields = BTreeMap::from([
        (
            "api_key".to_owned(),
            JsonData::String(MASTER_KEY_ALIAS.to_owned()),
        ),
        (
            "user_role".to_owned(),
            JsonData::String(UserRole::ProxyAdmin.to_string()),
        ),
        (
            "user_id".to_owned(),
            JsonData::String(admin_name.to_owned()),
        ),
    ]);
    let mut admin = UserAPIKeyAuth::construct(AuthInput::Dict(fields))?;
    admin.extra.provenance.via_virtual_key = true;
    Ok(Some(admin))
}
