use std::error::Error;

use secrecy::{ExposeSecret as _, SecretString};

use crate::database_tokens::endpoint::IAMEndpoint;
use crate::database_tokens::provider::AzureProviderFactory;
use crate::database_tokens::strategy::{
    DatabaseTokenAuth, DatabaseTokenError, mint_database_token,
};
use crate::database_url_policy::query::{add_missing_query_params, token_refresh_params_from_url};
use crate::database_url_policy::url_codec::UrlSplitError;

// Tests inject a minter so a refresh needs no AWS credential; the production one wraps B712.
pub trait TokenMinter: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn token(&self, endpoint: &IAMEndpoint) -> Result<Option<SecretString>, Self::Error>;
}

pub struct DatabaseTokenMinter<Factory>
where
    Factory: AzureProviderFactory,
{
    pub auth: Option<DatabaseTokenAuth<Factory>>,
}

impl<Factory> TokenMinter for DatabaseTokenMinter<Factory>
where
    Factory: AzureProviderFactory,
{
    type Error = DatabaseTokenError<Factory::Error>;

    fn token(&self, endpoint: &IAMEndpoint) -> Result<Option<SecretString>, Self::Error> {
        let Some(auth) = &self.auth else {
            return Ok(None);
        };
        mint_database_token(auth, endpoint).map(Some)
    }
}

// The old URL is parsed as it stands: only the refresh allowlist follows the new token URL.
pub fn assemble_refresh_url(
    endpoint: &IAMEndpoint,
    token: &SecretString,
    previous: Option<&SecretString>,
) -> Result<SecretString, UrlSplitError> {
    let assembled = endpoint.build_url(token.expose_secret());
    let params = token_refresh_params_from_url(previous.map_or("", |url| url.expose_secret()))?;
    add_missing_query_params(&assembled, &params).map(SecretString::from)
}
