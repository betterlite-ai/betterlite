use secrecy::SecretString;
use thiserror::Error;

use crate::azure_default_chain::environment::ChainEnvironment;
use crate::azure_default_chain::error::DefaultChainError;
use crate::azure_default_chain::grant::Grants;
use crate::azure_default_chain::provider::{
    AzureTokenProvider, ProviderRequest, get_azure_ad_token_provider,
};

use super::ENTRA_SCOPE;

#[derive(Debug, Error)]
pub enum MintError {
    #[error(transparent)]
    Chain(#[from] DefaultChainError),
    #[error("{detail}")]
    Detail { detail: String },
}

pub trait EntraTokenMinter: Send + Sync {
    fn mint(&self) -> Result<SecretString, MintError>;
}

pub struct NativeMinter<'grants, F: Grants> {
    pub(crate) provider: AzureTokenProvider<'grants, F>,
}

impl<'grants, F> EntraTokenMinter for NativeMinter<'grants, F>
where
    F: Grants + Send + Sync + 'grants,
    F::Grant<'grants>: Send + Sync,
{
    fn mint(&self) -> Result<SecretString, MintError> {
        let token = self.provider.token()?;
        Ok(token.token)
    }
}

pub fn default_entra_token_minter<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
) -> Result<NativeMinter<'grants, F>, MintError>
where
    F: Grants + 'grants,
{
    let request = ProviderRequest {
        scope: Some(ENTRA_SCOPE),
        credential: None,
        environment,
        grants,
    };
    let provider = get_azure_ad_token_provider(&request)?;
    Ok(NativeMinter { provider })
}
