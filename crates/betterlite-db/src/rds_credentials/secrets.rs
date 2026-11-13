use std::collections::BTreeMap;

use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use betterlite_secret_managers::secret_lookup::{SecretLookup, SecretValue};

pub trait RdsSecrets: Send + Sync {
    fn get_secret(&self, name: &str) -> Result<Option<SecretValue>, SecretLookupError>;
}

// get_secret reads the manager first and falls back to the same environment map the AWS SDK sees
pub struct EnvironmentSecrets<'lookup> {
    pub lookup: &'lookup SecretLookup,
    pub environment: &'lookup BTreeMap<String, String>,
}

impl RdsSecrets for EnvironmentSecrets<'_> {
    fn get_secret(&self, name: &str) -> Result<Option<SecretValue>, SecretLookupError> {
        self.lookup.get_secret(self.environment, name, None)
    }
}
