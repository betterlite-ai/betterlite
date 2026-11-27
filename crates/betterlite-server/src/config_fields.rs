pub mod error;
pub mod fields;
pub mod http;
pub mod litellm_fields;
pub mod models;
pub mod plugin_keys;
pub mod ports;
pub mod read;
pub mod records;
pub mod ui;
pub mod validation;
pub mod write;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::Router;
use tokio::sync::Mutex;

use crate::audit_events::sink::AuditEventSink;
use crate::config_overlay_port::ConfigOverlayPort;
use crate::plugin_routes::config::PluginRegistry;
use crate::proxy_config_state::state::ProxyConfigState;

pub use error::ConfigFieldError;
pub use ports::{
    ConfigFieldCaller, ConfigFieldDatabase, ConfigFieldRowError, ConfigFieldRows,
    ConfigFieldRuntime, ConfigFieldServices,
};
pub use read::{get_config_general_settings, list_config_general_settings};
pub use records::{
    ConfigFieldDelete, ConfigFieldInfo, ConfigFieldUpdate, ConfigFieldUpdateResponse, ConfigList,
    ConfigType, FieldDetail, GeneralSettingsRow, UiFieldResponse,
};
pub use write::{delete_config_general_settings, update_config_general_settings};

// what B8612 holds for the four routes: B8380's config state behind the lock its writes need, and the ports beside it
#[derive(Clone)]
pub struct ConfigFieldState {
    pub config: Arc<Mutex<ProxyConfigState>>,
    pub database: Option<ConfigFieldDatabase>,
    pub runtime: Arc<dyn ConfigFieldRuntime>,
    pub plugins: Arc<PluginRegistry>,
    pub audit: Arc<AuditEventSink>,
}

// the four /config routes; B8675 mounts this and the overlay supplies reload_config and the pass-through re-serve
pub fn routes(state: ConfigFieldState, overlay: Arc<dyn ConfigOverlayPort>) -> Router {
    http::router(state, overlay)
}
