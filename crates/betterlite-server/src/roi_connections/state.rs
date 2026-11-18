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
use super::gateway_http_client::GatewayHttpClient;

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
    let transport = match &state.github_transport {
        Some(injected) => injected.clone(),
        None => state.http.clone(),
    };
    GitHub::new(settings, transport)
}

pub(super) fn gateway<T: Clone>(state: &RoiConnectionsState<T>) -> GatewayHttpClient<T> {
    let base = state.gateway_base.clone();
    let transport = state.http.clone();
    GatewayHttpClient { base, transport }
}

pub(super) fn require_repository<T>(
    state: &RoiConnectionsState<T>,
) -> Result<&dyn SettingsRepository, RoiConnectionsError> {
    match state.repository.as_deref() {
        Some(repository) => Ok(repository),
        None => Err(RoiConnectionsError::Settings(RoiSettingsError::Internal(
            InternalError::DbNotConnected,
        ))),
    }
}
