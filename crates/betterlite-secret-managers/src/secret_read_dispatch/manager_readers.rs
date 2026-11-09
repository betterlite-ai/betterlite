use secrecy::SecretString;

use super::protocols::SyncSecretReader;
use crate::cyberark::CyberArkSecretManager;
use crate::cyberark::transport::ConjurTransport;
use crate::secret_contracts::BaseSecretManager;
use crate::secret_contracts::options::SecretOptions;
use crate::vault::HashicorpSecretManager;
use crate::vault::transport::VaultTransport;

// Both managers own the cached read, the namespace handling and the failure log LiteLLM performs
impl<T: VaultTransport> SyncSecretReader for HashicorpSecretManager<T> {
    fn sync_read_secret(&self, secret_name: &str) -> Option<SecretString> {
        <Self as BaseSecretManager>::sync_read_secret(self, secret_name, SecretOptions::default())
            .ok()
            .flatten()
    }
}

impl<T: ConjurTransport> SyncSecretReader for CyberArkSecretManager<T> {
    fn sync_read_secret(&self, secret_name: &str) -> Option<SecretString> {
        <Self as BaseSecretManager>::sync_read_secret(self, secret_name, SecretOptions::default())
            .ok()
            .flatten()
    }
}
