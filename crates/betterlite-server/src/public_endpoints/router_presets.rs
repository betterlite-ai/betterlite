pub mod catalog;
pub mod error;
pub mod scorer;
pub mod service;
mod tests;

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use betterlite_router::fuse_presets::get_fuse_presets;

use self::error::RouterPresetsError;
use self::scorer::ComplexityScorerDefaults;
use self::service::AutorouterPresetService;
use crate::error_body::json_error;
use crate::roi_github::transport::Transport;

pub const AUTOROUTER_PRESETS_PATH: &str = "/public/autorouter_presets";
pub const SCORER_DEFAULTS_PATH: &str = "/public/complexity_router/scorer_defaults";
pub const FUSE_PRESETS_PATH: &str = "/public/complexity_router/fuse_presets";

// one service behind all three paths keeps the catalog resolved once per app
pub fn router_presets_routes<Client>(service: Arc<AutorouterPresetService<Client>>) -> Router
where
    Client: Transport + 'static,
{
    Router::new()
        .route(
            AUTOROUTER_PRESETS_PATH,
            get(get_public_autorouter_presets::<Client>),
        )
        .route(SCORER_DEFAULTS_PATH, get(get_complexity_scorer_defaults))
        .route(FUSE_PRESETS_PATH, get(get_public_fuse_presets))
        .with_state(service)
}

async fn get_public_autorouter_presets<Client>(
    State(service): State<Arc<AutorouterPresetService<Client>>>,
) -> Response
where
    Client: Transport + 'static,
{
    match service.presets().await {
        Ok(presets) => Json(presets.as_ref()).into_response(),
        Err(error) => failed_response(&error),
    }
}

async fn get_complexity_scorer_defaults() -> Response {
    match ComplexityScorerDefaults::shipped() {
        Ok(defaults) => Json(defaults).into_response(),
        Err(error) => failed_response(&error),
    }
}

async fn get_public_fuse_presets() -> Response {
    match get_fuse_presets() {
        Ok(presets) => Json(presets.as_ref()).into_response(),
        Err(error) => failed_response(&RouterPresetsError::Fuse(error)),
    }
}

fn failed_response(error: &RouterPresetsError) -> Response {
    tracing::error!(error = %error, "a public router presets response could not be built");
    json_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error",
        "internal_server_error",
        "500",
    )
}
