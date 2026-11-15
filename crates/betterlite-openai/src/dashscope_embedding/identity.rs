use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;
use thiserror::Error;

// One DashScope brand: the base its calls start from, the key names it reads and whether an empty key counts as given
#[derive(Debug, Clone)]
pub struct FamilyIdentity {
    pub missing_key_message: String,
    pub default_base: String,
    pub env_base: Option<String>,
    pub env_key: Option<SecretString>,
    pub fallback_key: Option<SecretString>,
    pub empty_key_is_set: bool,
}

impl PartialEq for FamilyIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.missing_key_message == other.missing_key_message
            && self.default_base == other.default_base
            && self.env_base == other.env_base
            && self.empty_key_is_set == other.empty_key_is_set
            && same_secret(self.env_key.as_ref(), other.env_key.as_ref())
            && same_secret(self.fallback_key.as_ref(), other.fallback_key.as_ref())
    }
}

impl Eq for FamilyIdentity {}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FamilyKeyError {
    #[error("{message}")]
    Missing { message: String },
}

pub fn resolve_key(
    identity: &FamilyIdentity,
    api_key: Option<&str>,
) -> Result<SecretString, FamilyKeyError> {
    // LiteLLM's `or` chain skips an empty env value, and the plain identity takes an empty argument key as given
    let from_argument = api_key
        .filter(|key| identity.empty_key_is_set || !key.is_empty())
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
        .ok_or_else(|| FamilyKeyError::Missing {
            message: identity.missing_key_message.clone(),
        })
}

fn same_secret(left: Option<&SecretString>, right: Option<&SecretString>) -> bool {
    match (left, right) {
        (Some(left_key), Some(right_key)) => bool::from(
            left_key
                .expose_secret()
                .as_bytes()
                .ct_eq(right_key.expose_secret().as_bytes()),
        ),
        (None, None) => true,
        _ => false,
    }
}
