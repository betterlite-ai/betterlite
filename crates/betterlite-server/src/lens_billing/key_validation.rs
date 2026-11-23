use async_trait::async_trait;
use betterlite_auth::auth_identity::errors::AuthError;
use thiserror::Error;

const INACTIVE_DETAIL: &str = "Choose an active virtual key for Lens analysis";

#[derive(Debug, Error)]
pub enum KeyValidationError {
    #[error("{INACTIVE_DETAIL}")]
    Inactive,
    #[error(transparent)]
    Resolve(#[from] AuthError),
}

impl KeyValidationError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::Inactive => 400,
            Self::Resolve(error) => match error.http_mapping() {
                Some(mapping) => mapping.status,
                None => 500,
            },
        }
    }

    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Self::Inactive => INACTIVE_DETAIL.to_owned(),
            Self::Resolve(error) => error.to_string(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct LensKey {
    pub blocked: bool,
    pub is_session_token: bool,
    pub team_id: Option<String>,
}

#[async_trait]
pub trait KeyResolver: Send + Sync {
    async fn resolve(&self, hashed_token: &str) -> Result<LensKey, AuthError>;
}

pub async fn validate_key(
    resolver: &dyn KeyResolver,
    key_id: Option<&str>,
) -> Result<Option<LensKey>, KeyValidationError> {
    let Some(hashed_token) = key_id else {
        return Ok(None);
    };
    let key = resolver.resolve(hashed_token).await?;
    if key.blocked || key.is_session_token {
        return Err(KeyValidationError::Inactive);
    }
    Ok(Some(key))
}

mod tests;
