use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use crate::auth::auth_identity::AuthContext;
use crate::config_overlay_port::ConfigOverlayPort;
use crate::proxy_config_state::state::ProxyConfigState;

use super::ConfigFieldState;
use super::error::ConfigFieldError;
use super::ports::{ConfigFieldCaller, ConfigFieldServices};
use super::read::{get_config_general_settings, list_config_general_settings};
use super::records::{ConfigFieldDelete, ConfigFieldUpdate, GENERAL_SETTINGS};
use super::write::{delete_config_general_settings, update_config_general_settings};

const INVALID_BODY: &str = "Invalid request body";
const FIELD_REQUIRED: &str = "Field required";
const LITERAL_ERROR: &str = "Input should be 'general_settings'";

#[derive(Clone)]
pub(super) struct HttpState {
    pub(super) fields: ConfigFieldState,
    pub(super) overlay: Arc<dyn ConfigOverlayPort>,
}

#[derive(Deserialize)]
struct InfoQuery {
    field_name: Option<String>,
}

#[derive(Deserialize)]
struct ListQuery {
    config_type: Option<String>,
}

#[derive(Serialize)]
struct ErrorBody<'text> {
    detail: &'text str,
}

pub(super) fn router(state: ConfigFieldState, overlay: Arc<dyn ConfigOverlayPort>) -> Router {
    Router::new()
        .route("/config/field/update", post(update))
        .route("/config/field/info", get(info))
        .route("/config/list", get(list))
        .route("/config/field/delete", post(delete))
        .with_state(HttpState {
            fields: state,
            overlay,
        })
}

async fn update(
    State(state): State<HttpState>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Response {
    let Ok(update) = serde_json::from_slice::<ConfigFieldUpdate>(&body) else {
        return unprocessable(INVALID_BODY);
    };
    let client = match client(caller.as_ref()) {
        Ok(client) => client,
        Err(error) => return error.into_response(),
    };
    let mut config = state.fields.config.lock().await;
    let services = services(&state, &mut config);
    match update_config_general_settings(services, &client, &update).await {
        Ok(body) => Json(body).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn delete(
    State(state): State<HttpState>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Response {
    let Ok(delete) = serde_json::from_slice::<ConfigFieldDelete>(&body) else {
        return unprocessable(INVALID_BODY);
    };
    let client = match client(caller.as_ref()) {
        Ok(client) => client,
        Err(error) => return error.into_response(),
    };
    let mut config = state.fields.config.lock().await;
    let services = services(&state, &mut config);
    match delete_config_general_settings(services, &client, &delete).await {
        Ok(body) => Json(body).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn info(
    State(state): State<HttpState>,
    caller: Option<Extension<AuthContext>>,
    Query(query): Query<InfoQuery>,
) -> Response {
    let Some(field_name) = query.field_name else {
        return unprocessable(FIELD_REQUIRED);
    };
    let client = match client(caller.as_ref()) {
        Ok(client) => client,
        Err(error) => return error.into_response(),
    };
    let mut config = state.fields.config.lock().await;
    let services = services(&state, &mut config);
    match get_config_general_settings(services, &client, &field_name).await {
        Ok(body) => Json(body).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn list(
    State(state): State<HttpState>,
    caller: Option<Extension<AuthContext>>,
    Query(query): Query<ListQuery>,
) -> Response {
    let Some(config_type) = query.config_type else {
        return unprocessable(FIELD_REQUIRED);
    };
    if config_type != GENERAL_SETTINGS {
        return unprocessable(LITERAL_ERROR);
    }
    let client = match client(caller.as_ref()) {
        Ok(client) => client,
        Err(error) => return error.into_response(),
    };
    let mut config = state.fields.config.lock().await;
    let services = services(&state, &mut config);
    match list_config_general_settings(services, &client).await {
        Ok(body) => Json(body).into_response(),
        Err(error) => error.into_response(),
    }
}

fn client(caller: Option<&Extension<AuthContext>>) -> Result<ConfigFieldCaller, ConfigFieldError> {
    let Some(Extension(context)) = caller else {
        return Err(ConfigFieldError::Unauthenticated);
    };
    ConfigFieldCaller::from_principal(&context.principal)
        .map_err(|_error| ConfigFieldError::Internal)
}

fn services<'run>(
    state: &'run HttpState,
    config: &'run mut ProxyConfigState,
) -> ConfigFieldServices<'run> {
    ConfigFieldServices {
        state: config,
        database: state.fields.database.as_ref(),
        runtime: state.fields.runtime.as_ref(),
        plugins: state.fields.plugins.as_ref(),
        audit: state.fields.audit.as_ref(),
        overlay: state.overlay.as_ref(),
    }
}

fn unprocessable(message: &'static str) -> Response {
    let status = StatusCode::UNPROCESSABLE_ENTITY;
    (status, Json(ErrorBody { detail: message })).into_response()
}
