use std::sync::Arc;

use secrecy::SecretString;
use thiserror::Error;

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::oidc_exchange::OidcExchangeInput;

#[derive(Debug, Clone)]
pub enum TokenReply {
    Text(SecretString),
    NonText { type_name: String },
}

#[derive(Debug, Clone)]
pub struct ProviderFailure {
    pub class: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct SourceFailure {
    pub class: String,
    pub status: Option<u16>,
    pub message: String,
}

pub trait CallableTokenProvider: Send + Sync {
    fn token(&self) -> Result<TokenReply, ProviderFailure>;
}

pub trait CredentialSources {
    fn client_secret<'source>(
        &'source self,
        record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure>;

    fn username_password<'source>(
        &'source self,
        record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure>;

    fn sdk_refresh<'source>(
        &'source self,
        scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure>;

    fn default_credential<'source>(
        &'source self,
        scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>>;

    fn oidc(&self, input: &OidcExchangeInput) -> Result<SecretString, SourceFailure>;
}
