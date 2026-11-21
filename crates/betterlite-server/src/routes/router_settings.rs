use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use betterlite_config::types::Config;

use crate::router_settings::response::{
    RouterFieldsResponse, RouterSettingsResponse, fields_response, settings_response,
};
use crate::router_settings::view::{current_values, settings_store};
use crate::state::ServerState;

// the settings routes read the loaded config, so the router they build holds no request state
#[derive(Clone)]
struct SettingsState {
    config: Arc<Config>,
}

#[cfg_attr(test, mutants::skip)]
pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    Router::new()
        .route("/router/settings", get(settings))
        .route("/router/fields", get(fields))
        .with_state(SettingsState {
            config: Arc::new(state.config.clone()),
        })
}

#[cfg_attr(test, mutants::skip)]
async fn settings(State(state): State<SettingsState>) -> Json<RouterSettingsResponse> {
    let store = settings_store(&state.config);
    Json(settings_response(current_values(&state.config), &store))
}

#[cfg_attr(test, mutants::skip)]
async fn fields() -> Json<RouterFieldsResponse> {
    Json(fields_response())
}
