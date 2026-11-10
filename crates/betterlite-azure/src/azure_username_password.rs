use std::io;

use secrecy::SecretString;
use thiserror::Error;

use crate::azure_client_secret::ClientSecretError;

pub mod authority;
pub mod provider;
pub mod record;
pub mod scope;
pub mod token;
pub mod transport;

mod tests;

pub const DEFAULT_SCOPE: &str = "https://cognitiveservices.azure.com/.default";
pub const DEFAULT_AUTHORITY_HOST: &str = "login.microsoftonline.com";

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UsernamePasswordInput {
    pub client_id: String,
    pub username: String,
    pub password: SecretString,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Error)]
pub enum UsernamePasswordError {
    #[error(transparent)]
    Authority(#[from] ClientSecretError),
    #[error(transparent)]
    Token(#[from] token::PasswordTokenError),
    #[error("Authentication failed: {message}")]
    Authentication { message: String },
    #[error("Authentication failed: {source}")]
    Transport {
        #[source]
        source: io::Error,
    },
    #[error("Authentication failed: You cannot use any scope value that is reserved.")]
    ReservedScope,
    #[error(
        "Authentication failed: the account is federated, and the WS-Trust sign-in it needs is not supported"
    )]
    FederatedAccount,
    #[error("the user realm response is not the expected JSON")]
    Realm(#[source] serde_json::Error),
    #[error("the token lifetime overflows the clock")]
    ExpiryOverflow,
    #[error("the token provider cache is poisoned")]
    ProviderCachePoisoned,
}
