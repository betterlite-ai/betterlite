use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Extension, Query, State};
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
use super::records::{
    ConfigFieldDelete, ConfigFieldUpdate, ConfigFieldUpdateResponse, ConfigList, GENERAL_SETTINGS,
};
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
) -> Result<Json<ConfigFieldUpdateResponse>, ConfigFieldError> {
    let update: ConfigFieldUpdate =
        serde_json::from_slice(&body).map_err(|_error| ConfigFieldError::Unprocessable {
            detail: INVALID_BODY,
        })?;
    let client = caller_of(caller.as_ref())?;
    let mut config = state.fields.config.lock().await;
    let services = services(&state, &mut config);
    let row = update_config_general_settings(services, &client, &update).await?;
    Ok(Json(row))
}

async fn delete(
    State(state): State<HttpState>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Response {
    let Ok(delete) = serde_json::from_slice::<ConfigFieldDelete>(&body) else {
        return ConfigFieldError::Unprocessable {
            detail: INVALID_BODY,
        }
        .into_response();
    };
    let client = match caller_of(caller.as_ref()) {
        Ok(client) => client,
        Err(error) => return error.into_response(),
    };
    let mut config = state.fields.config.lock().await;
    respond(delete_config_general_settings(services(&state, &mut config), &client, &delete).await)
}

async fn info(
    State(state): State<HttpState>,
    caller: Option<Extension<AuthContext>>,
    Query(query): Query<InfoQuery>,
) -> Response {
    let Some(field_name) = query.field_name else {
        return ConfigFieldError::Unprocessable {
            detail: FIELD_REQUIRED,
        }
        .into_response();
    };
    match caller_of(caller.as_ref()) {
        Ok(client) => {
            let mut config = state.fields.config.lock().await;
            let info =
                get_config_general_settings(services(&state, &mut config), &client, &field_name);
            respond(info.await)
        }
        Err(error) => error.into_response(),
    }
}

async fn list(
    State(state): State<HttpState>,
    caller: Option<Extension<AuthContext>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<ConfigList>>, ConfigFieldError> {
    let Some(config_type) = query.config_type else {
        return Err(ConfigFieldError::Unprocessable {
            detail: FIELD_REQUIRED,
        });
    };
    if config_type != GENERAL_SETTINGS {
        return Err(ConfigFieldError::Unprocessable {
            detail: LITERAL_ERROR,
        });
    }
    let client = caller_of(caller.as_ref())?;
    let mut config = state.fields.config.lock().await;
    let fields = list_config_general_settings(services(&state, &mut config), &client).await?;
    Ok(Json(fields))
}

// user_api_key_dict for these routes: the caller the role gates and the config audit log read
fn caller_of(
    caller: Option<&Extension<AuthContext>>,
) -> Result<ConfigFieldCaller, ConfigFieldError> {
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

fn respond<T: Serialize>(result: Result<T, ConfigFieldError>) -> Response {
    match result {
        Ok(body) => Json(body).into_response(),
        Err(error) => error.into_response(),
    }
}
