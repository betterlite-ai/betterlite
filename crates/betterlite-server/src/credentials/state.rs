use std::sync::Arc;

use betterlite_db::stored_cipher::DbCipher;

use super::lookup::DeploymentLookup;
use super::registry::CredentialRegistry;
use super::store::CredentialStore;

#[derive(Clone)]
pub struct CredentialState {
    pub registry: Arc<CredentialRegistry>,
    pub cipher: Arc<DbCipher>,
    pub store: Option<Arc<dyn CredentialStore>>,
    pub lookup: Option<Arc<dyn DeploymentLookup>>,
}
