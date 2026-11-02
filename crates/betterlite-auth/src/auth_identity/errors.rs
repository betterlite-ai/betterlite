use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("no database connection is configured for identity resolution")]
    NoDatabaseConnection,
    #[error("key {hashed_token} was not in the cache and the lookup was cache-only")]
    KeyNotInCache { hashed_token: String },
    #[error("key {hashed_token} was not found")]
    KeyNotFound { hashed_token: String },
    #[error("the principal carries no source key; it was not produced by identity resolution")]
    PrincipalMissingSourceKey,
    #[error("user {user_id} does not exist")]
    UserNotFound { user_id: String },
}

pub struct TokenNotFoundInDb {
    pub status: u16,
    pub error_type: &'static str,
    pub param: &'static str,
    pub code: &'static str,
    pub headers: &'static [(&'static str, &'static str)],
}

impl AuthError {
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NoDatabaseConnection => "no_database_connection",
            Self::KeyNotInCache { .. } => "key_not_in_cache",
            Self::KeyNotFound { .. } => "key_not_found",
            Self::PrincipalMissingSourceKey => "principal_missing_source_key",
            Self::UserNotFound { .. } => "user_not_found",
        }
    }

    #[must_use]
    pub const fn http_mapping(&self) -> Option<TokenNotFoundInDb> {
        match self {
            Self::KeyNotFound { .. } => Some(TokenNotFoundInDb {
                status: 401,
                error_type: "token_not_found_in_db",
                param: "key",
                code: "401",
                headers: &[],
            }),
            Self::NoDatabaseConnection
            | Self::KeyNotInCache { .. }
            | Self::PrincipalMissingSourceKey
            | Self::UserNotFound { .. } => None,
        }
    }
}

mod tests;
