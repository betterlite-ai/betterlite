use std::collections::BTreeMap;

use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use secrecy::SecretString;

use betterlite_provider::billing::OPENAI_ADMIN_KEY_ENV_VAR;

#[derive(Debug, thiserror::Error)]
pub enum BillingCredentialError {
    #[error(transparent)]
    Lookup(#[from] SecretLookupError),
}

// get_secret_str of the source: the configured secret manager first, then the environment.
pub trait BillingCredentials: Send + Sync {
    fn admin_key(&self) -> Result<Option<SecretString>, BillingCredentialError>;
}

pub struct SecretBillingCredentials<'env> {
    pub lookup: &'env SecretLookup,
    pub environment: &'env BTreeMap<String, String>,
}

impl BillingCredentials for SecretBillingCredentials<'_> {
    fn admin_key(&self) -> Result<Option<SecretString>, BillingCredentialError> {
        let key = self
            .lookup
            .get_secret_str(self.environment, OPENAI_ADMIN_KEY_ENV_VAR, None)?;
        Ok(key)
    }
}
