use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;

use super::error::{EndpointInstallError, OverlayError};
use super::state::{OverlayState, PASS_THROUGH_ENDPOINTS};

// initialize_pass_through_endpoints behind a port: it appends the config file's list itself, so it gets the database list alone
#[async_trait]
pub trait PassThroughInstaller: Send + Sync {
    async fn install(&self, endpoints: &[JsonValue]) -> Result<(), EndpointInstallError>;
}

// _pass_through_endpoints_beside_db: the database entries, then each file entry whose path the database lacks
#[must_use]
pub fn endpoints_beside_db(
    db_endpoints: &[JsonValue],
    config_endpoints: &[JsonValue],
) -> Vec<JsonValue> {
    let path_of = |endpoint: &JsonValue| endpoint.get("path").cloned().unwrap_or_default();
    let db_paths: Vec<JsonValue> = db_endpoints
        .iter()
        .filter(|endpoint| endpoint.is_object())
        .map(path_of)
        .collect();
    let beside_db = config_endpoints
        .iter()
        .filter(|&endpoint| !endpoint.is_object() || !db_paths.contains(&path_of(endpoint)));
    db_endpoints.iter().chain(beside_db).cloned().collect()
}

// _publish_pass_through_endpoints: the store serves the database list beside the startup config list
pub fn publish_pass_through(
    state: &mut OverlayState,
    db_endpoints: &[JsonValue],
) -> Result<(), OverlayError> {
    let published = endpoints_beside_db(db_endpoints, &state.startup_pass_through);
    let general_settings = &mut state.config.general_settings;
    general_settings.insert(PASS_THROUGH_ENDPOINTS, JsonValue::Array(published))?;
    Ok(())
}

pub fn validate_db_endpoints(db_endpoints: &[JsonValue]) -> Result<(), OverlayError> {
    if let Some(index) = db_endpoints
        .iter()
        .position(|endpoint| !endpoint.is_object())
    {
        return Err(OverlayError::PassThroughEntry { index });
    }
    Ok(())
}

pub async fn install_pass_through_endpoints(
    db_endpoints: &[JsonValue],
    installer: &dyn PassThroughInstaller,
) -> Result<(), OverlayError> {
    installer
        .install(db_endpoints)
        .await
        .map_err(|source| OverlayError::PassThroughInstall { source })
}

// _serve_pass_through_endpoints: the new list is published before it is checked, so a refused list still shows in the store
pub async fn serve_pass_through_endpoints(
    state: &mut OverlayState,
    db_endpoints: &[JsonValue],
    installer: &dyn PassThroughInstaller,
) -> Result<(), OverlayError> {
    publish_pass_through(state, db_endpoints)?;
    validate_db_endpoints(db_endpoints)?;
    install_pass_through_endpoints(db_endpoints, installer).await
}
