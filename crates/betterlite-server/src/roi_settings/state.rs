use std::sync::Arc;

use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::CipherSettings;
use betterlite_secrets::secret_codec::OsNonces;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;

use super::deployments::RouterCatalog;
use super::repository::SettingsRepository;

#[derive(Clone)]
pub struct RoiSettingsState {
    pub(super) repository: Option<Arc<dyn SettingsRepository>>,
    pub(super) catalog: Option<Arc<dyn RouterCatalog>>,
    pub(super) cipher: Arc<DbCipher>,
}

impl RoiSettingsState {
    #[must_use]
    pub fn new(
        repository: Option<Arc<dyn SettingsRepository>>,
        catalog: Option<Arc<dyn RouterCatalog>>,
        cipher_settings: CipherSettings,
    ) -> Self {
        Self {
            repository,
            catalog,
            cipher: Arc::new(DbCipher {
                settings: cipher_settings,
                nonces: OsNonces,
                diagnostics: TracingDiagnostics,
            }),
        }
    }
}
