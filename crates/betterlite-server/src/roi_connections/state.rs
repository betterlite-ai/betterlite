use std::sync::Arc;

use betterlite_roi::settings::ROISettings;
use secrecy::SecretString;
use url::Url;

use crate::roi_github::client::GitHub;
use crate::roi_github::error::SourceError;
use crate::roi_github::transport::Transport;
use crate::roi_settings::deployments::RouterCatalog;
use crate::roi_settings::error::{InternalError, RoiSettingsError};
use crate::roi_settings::repository::SettingsRepository;
use crate::roi_settings::store::SettingsCipher;

use super::error::RoiConnectionsError;

#[derive(Clone)]
pub struct RoiConnectionsState<T> {
    pub repository: Option<Arc<dyn SettingsRepository>>,
    pub catalog: Option<Arc<dyn RouterCatalog>>,
    pub cipher: Arc<dyn SettingsCipher>,
    pub master_key: Option<SecretString>,
    pub http: T,
    pub github_transport: Option<T>,
    pub gateway_base: Url,
}

pub(super) fn github_client<T>(
    settings: &ROISettings,
    state: &RoiConnectionsState<T>,
) -> Result<GitHub<T>, SourceError>
where
    T: Transport + Clone,
{
    GitHub::new(
        settings,
        state
            .github_transport
            .clone()
            .unwrap_or_else(|| state.http.clone()),
    )
}

pub(super) fn require_repository<T>(
    state: &RoiConnectionsState<T>,
) -> Result<&dyn SettingsRepository, RoiConnectionsError> {
    state.repository.as_deref().ok_or_else(|| {
        RoiConnectionsError::Settings(RoiSettingsError::Internal(InternalError::DbNotConnected))
    })
}
