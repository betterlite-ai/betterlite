use tokio::runtime::Builder;
use tokio::sync::Mutex;

use betterlite_secrets::gateway_key::{GatewayKeyStore, StoredCredential};

use crate::keyring::SecretVault;

use super::files::CredentialFiles;
use super::load::resolve_secret;

pub struct VaultGatewayStore<'store, V: SecretVault + Send + Sync> {
    pub files: &'store dyn CredentialFiles,
    pub vault: &'store Mutex<V>,
}

impl<V: SecretVault + Send + Sync> GatewayKeyStore for VaultGatewayStore<'_, V> {
    fn stored_credential(&self, expected_base_url: Option<&str>) -> Option<StoredCredential> {
        let record = self.files.read()?;
        if let Some(expected) = expected_base_url
            && record.base_url != expected.trim_end_matches('/')
        {
            return Some(StoredCredential {
                base_url: record.base_url,
                key: None,
            });
        }
        // the SDK entry point is synchronous and the token file operations are not
        let runtime = Builder::new_current_thread().build().ok()?;
        let resolved = runtime.block_on(async {
            let mut vault = self.vault.lock().await;
            resolve_secret(self.files, &mut *vault, record).await
        })?;
        Some(StoredCredential {
            base_url: resolved.base_url,
            key: resolved.key,
        })
    }
}
