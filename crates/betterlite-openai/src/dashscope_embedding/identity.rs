use thiserror::Error;

// One DashScope brand: the base its calls start from, the key names it reads and whether an empty key counts as given
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyIdentity {
    pub missing_key_message: String,
    pub default_base: String,
    pub env_base: Option<String>,
    pub env_key: Option<String>,
    pub fallback_key: Option<String>,
    pub empty_key_is_set: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FamilyKeyError {
    #[error("{message}")]
    Missing { message: String },
}

pub fn resolve_key(
    identity: &FamilyIdentity,
    api_key: Option<&str>,
) -> Result<String, FamilyKeyError> {
    // LiteLLM's `or` chain skips an empty env value, and the plain identity takes an empty argument key as given
    let from_argument = api_key.filter(|key| identity.empty_key_is_set || !key.is_empty());
    let from_env = identity.env_key.as_deref().filter(|key| !key.is_empty());
    let from_fallback = identity
        .fallback_key
        .as_deref()
        .filter(|key| !key.is_empty());
    from_argument
        .or(from_env)
        .or(from_fallback)
        .map(str::to_owned)
        .ok_or_else(|| FamilyKeyError::Missing {
            message: identity.missing_key_message.clone(),
        })
}
