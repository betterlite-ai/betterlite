use secrecy::{ExposeSecret as _, SecretString};

use super::error::DashscopeImageError;

// One DashScope image brand: the key names it reads and the base its calls start from
#[derive(Debug, Clone)]
pub struct ImageIdentity {
    pub missing_key_message: String,
    pub env_key: Option<SecretString>,
    pub fallback_key: Option<SecretString>,
    pub env_image_base: Option<String>,
    pub default_image_base: String,
}

pub fn resolve_image_key(
    identity: &ImageIdentity,
    api_key: Option<&str>,
) -> Result<SecretString, DashscopeImageError> {
    // LiteLLM's `or` chain skips an empty argument and an empty environment value
    let from_argument = api_key
        .filter(|key| !key.is_empty())
        .map(SecretString::from);
    let from_env = identity
        .env_key
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty())
        .cloned();
    let from_fallback = identity
        .fallback_key
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty())
        .cloned();
    from_argument
        .or(from_env)
        .or(from_fallback)
        .ok_or_else(|| DashscopeImageError::MissingKey {
            message: identity.missing_key_message.clone(),
        })
}
