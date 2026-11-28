pub mod alerts;
pub mod body;
pub mod callbacks;
pub mod delete;
pub mod error;
pub mod get;
pub mod hidden;
pub mod ports;

mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::Extension;
use axum::response::{IntoResponse as _, Response};
use axum::routing::{get as get_method, post};
use serde::Serialize;

use crate::audit_events::sink::AuditEventSink;
use crate::auth::auth_identity::AuthContext;
use crate::cache_settings_update::route::StoreModelInDbSettings;
use crate::config_overlay_port::ConfigOverlayPort;
use crate::deployment_sync_port::DeploymentSyncPort;
use crate::tracing_routes::router::unauthenticated;

use self::ports::{
    AlertingState, CallbackClock, ConfigSaver, RouterSettingsReader, RuntimeCallbacks,
};

// Everything the callback config routes read, built by the mount that owns the live proxy state.
#[derive(Clone)]
pub struct CallbackConfigState {
    pub overlay: Arc<dyn ConfigOverlayPort>,
    pub saver: Arc<dyn ConfigSaver>,
    pub database_connected: bool,
    pub store_model_in_db: Arc<dyn StoreModelInDbSettings>,
    pub callbacks: Arc<dyn RuntimeCallbacks>,
    pub alerting: Arc<dyn AlertingState>,
    pub router: Option<Arc<dyn RouterSettingsReader>>,
    pub environment: Arc<BTreeMap<String, String>>,
    pub audit: Arc<AuditEventSink>,
    pub clock: Arc<dyn CallbackClock>,
}

#[derive(Clone)]
pub(crate) struct HandlerState {
    pub(crate) config: CallbackConfigState,
    pub(crate) deployments: Arc<dyn DeploymentSyncPort>,
}

pub fn routes(state: CallbackConfigState, deployments: Arc<dyn DeploymentSyncPort>) -> Router {
    let handler = HandlerState {
        config: state,
        deployments,
    };
    Router::new()
        .route("/config/callback/delete", post(delete::delete_callback))
        .route("/get/config/callbacks", get_method(get::get_config))
        .route("/config/yaml", get_method(config_yaml))
        .with_state(handler)
}

#[derive(Serialize)]
struct HelloWorld {
    hello: &'static str,
}

// config_yaml_endpoint: a mock that documents ConfigYAML, behind LiteLLM's user_api_key_auth dependency and body validation
async fn config_yaml(caller: Option<Extension<AuthContext>>, payload: Bytes) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    match body::config_yaml_request(&payload) {
        Ok(()) => Json(HelloWorld { hello: "world" }).into_response(),
        Err(error) => error.into_response(),
    }
}
