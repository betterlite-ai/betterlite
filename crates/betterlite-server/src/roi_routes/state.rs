use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::roi_settings::deployments::RouterCatalog;
use crate::roi_settings::repository::SettingsRepository;
use crate::roi_settings::store::SettingsCipher;
use crate::roi_sync::ports::UuidSource;

use super::ports::{CompletionFactory, SpendReaderFactory, SyncManagerPort, SyncStoreSource};

#[derive(Clone)]
pub struct RoiRouteState {
    pub repository: Option<Arc<dyn SettingsRepository>>,
    pub cipher: Arc<dyn SettingsCipher>,
    pub catalog: Option<Arc<dyn RouterCatalog>>,
    pub sync_store: Arc<dyn SyncStoreSource>,
    pub manager: Arc<dyn SyncManagerPort>,
    pub clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    pub spend_reader: SpendReaderFactory,
    pub completion_caller: CompletionFactory,
    pub next_id: UuidSource,
}
