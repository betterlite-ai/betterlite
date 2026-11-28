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

mod tests;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use tokio::sync::Mutex;

use crate::audit_events::sink::AuditEventSink;
use crate::config_overlay_port::ConfigOverlayPort;
use crate::plugin_routes::config::PluginRegistry;
use crate::proxy_config_state::state::ProxyConfigState;

use self::ports::{ConfigFieldDatabase, ConfigFieldRuntime};

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
    let http_state = http::HttpState {
        fields: state,
        overlay,
    };
    Router::new()
        .route("/config/field/update", post(http::update))
        .route("/config/field/info", get(http::info))
        .route("/config/list", get(http::list))
        .route("/config/field/delete", post(http::delete))
        .with_state(http_state)
}
