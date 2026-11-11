use secrecy::SecretString;

use crate::azure_client_secret::SecretResolver;
use crate::oidc_exchange::{AzureOidcExchange, OidcExchangeInput};

use super::error::ManagedIdentityError;
use super::runtime::TokenFiles;
use super::selection::WorkloadSelection;

// the local token file is already the assertion, so resolving it a second time returns the same bytes
struct FileAssertion(SecretString);

impl SecretResolver for FileAssertion {
    fn get_secret_str(&self, _reference: &str) -> Option<SecretString> {
        Some(self.0.clone())
    }
}

pub fn acquire(
    exchange: &AzureOidcExchange,
    files: &dyn TokenFiles,
    workload: &WorkloadSelection,
    scope: &str,
) -> Result<SecretString, ManagedIdentityError> {
    let bytes = files
        .read(&workload.token_file)
        .map_err(|source| ManagedIdentityError::TokenFile { source })?;
    let assertion = String::from_utf8(bytes)
        .map_err(|source| ManagedIdentityError::TokenFileEncoding { source })?;
    let input = OidcExchangeInput {
        azure_ad_token: SecretString::from(assertion.clone()),
        azure_client_id: Some(workload.client_id.clone()),
        azure_tenant_id: Some(workload.tenant_id.clone()),
        scope: Some(scope.to_owned()),
    };
    let token = exchange.token(&input, &FileAssertion(SecretString::from(assertion)))?;
    Ok(token)
}
