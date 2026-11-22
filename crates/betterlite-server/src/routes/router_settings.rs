use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use betterlite_config::types::Config;

use crate::router_settings::response::{fields_response, settings_response};
use crate::router_settings::view::{current_values, settings_store};
use crate::router_settings::{FIELDS_PATH, SETTINGS_PATH};
use crate::state::ServerState;

// the settings routes read the loaded config, so the router they build holds no request state
#[derive(Clone)]
struct SettingsState {
    config: Arc<Config>,
}

#[cfg_attr(test, mutants::skip)]
pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    Router::new()
        .route(
            SETTINGS_PATH,
            get(|State(settings): State<SettingsState>| async move {
                Json(settings_response(
                    current_values(&settings.config),
                    &settings_store(&settings.config),
                ))
            }),
        )
        .route(FIELDS_PATH, get(|| async { Json(fields_response()) }))
        .with_state(SettingsState {
            config: Arc::new(state.config.clone()),
        })
}
