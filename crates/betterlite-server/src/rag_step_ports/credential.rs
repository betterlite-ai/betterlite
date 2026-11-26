use std::fmt::{self, Formatter};
use std::sync::Arc;

use betterlite_config::credentials::CredentialValues;
use betterlite_core::rag::ingestion::credentials::{
    CredentialLookup as CredentialLookupPort, CredentialLookupError,
};

use crate::credentials::registry::CredentialRegistry;

mod tests;

// the CredentialAccessor lookup over the store B1100 fills; the registry holds the credential values
pub struct CredentialLookup {
    pub registry: Arc<CredentialRegistry>,
}

impl CredentialLookupPort for CredentialLookup {
    fn credential_values(
        &self,
        credential_name: &str,
    ) -> Result<CredentialValues, CredentialLookupError> {
        let Some(credential) = self.registry.find_credential(credential_name) else {
            return Err(CredentialLookupError::NotFound {
                credential_name: credential_name.to_owned(),
            });
        };
        Ok(credential.credential_values)
    }
}

impl fmt::Debug for CredentialLookup {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialLookup")
            .finish_non_exhaustive()
    }
}
