use std::sync::Arc;

use super::deployments::RouterCatalog;
use super::repository::SettingsRepository;
use super::store::SettingsCipher;

#[derive(Clone)]
pub struct RoiSettingsState {
    pub repository: Option<Arc<dyn SettingsRepository>>,
    pub catalog: Option<Arc<dyn RouterCatalog>>,
    pub cipher: Arc<dyn SettingsCipher>,
}
