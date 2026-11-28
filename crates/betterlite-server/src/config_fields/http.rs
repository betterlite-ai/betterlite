use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Query, State};
use axum::response::{IntoResponse as _, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::auth::auth_identity::AuthContext;
use crate::config_overlay_port::ConfigOverlayPort;
use crate::proxy_config_state::state::ProxyConfigState;

use super::ConfigFieldState;
use super::error::ConfigFieldError;
use super::ports::{ConfigFieldCaller, ConfigFieldServices};
use super::read::{get_config_general_settings, list_config_general_settings};
use super::records::{ConfigFieldDelete, ConfigFieldUpdate, ConfigList, GENERAL_SETTINGS};
use super::write::{delete_config_general_settings, update_config_general_settings};

const INVALID_BODY: &str = "Invalid request body";
const FIELD_REQUIRED: &str = "Field required";
const LITERAL_ERROR: &str = "Input should be 'general_settings'";

#[derive(Clone)]
pub(super) struct HttpState {
    pub(super) fields: ConfigFieldState,
    pub(super) overlay: Arc<dyn ConfigOverlayPort>,
}

// user_api_key_dict for these routes: optional because FastAPI answers 401 when it is missing
pub(super) type Auth = Option<Extension<AuthContext>>;

#[derive(Deserialize)]
pub(super) struct InfoQuery {
    field_name: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ListQuery {
    config_type: Option<String>,
}

// the decoded body each write route carries, and the catalog edit it asks for
enum FieldWrite {
    Update(ConfigFieldUpdate),
    Delete(ConfigFieldDelete),
}

// the body decode both write routes run; the fuzz target and its corpus replay feed it arbitrary bytes
pub fn decode_body<T>(body: &[u8]) -> Result<T, ConfigFieldError>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(body).map_err(|_error| ConfigFieldError::Unprocessable {
        detail: INVALID_BODY,
    })
}

pub(super) async fn update(State(state): State<HttpState>, auth: Auth, body: Bytes) -> Response {
    let request = match decode_body::<ConfigFieldUpdate>(&body) {
        Ok(update) => FieldWrite::Update(update),
        Err(error) => return error.into_response(),
    };
    respond_write(&state, auth.as_ref(), request).await
}

pub(super) async fn delete(State(state): State<HttpState>, auth: Auth, body: Bytes) -> Response {
    let request = match decode_body::<ConfigFieldDelete>(&body) {
        Ok(delete) => FieldWrite::Delete(delete),
        Err(error) => return error.into_response(),
    };
    respond_write(&state, auth.as_ref(), request).await
}

pub(super) async fn info(
    State(state): State<HttpState>,
    auth: Auth,
    Query(query): Query<InfoQuery>,
) -> Response {
    let Some(field_name) = query.field_name else {
        return ConfigFieldError::Unprocessable {
            detail: FIELD_REQUIRED,
        }
        .into_response();
    };
    match caller_of(auth.as_ref()) {
        Ok(client) => {
            let info = {
                let mut config = state.fields.config.lock().await;
                get_config_general_settings(services(&state, &mut config), &client, &field_name)
                    .await
            };
            respond(info)
        }
        Err(error) => error.into_response(),
    }
}

pub(super) async fn list(
    State(state): State<HttpState>,
    auth: Auth,
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
    let client = caller_of(auth.as_ref())?;
    let fields = {
        let mut config = state.fields.config.lock().await;
        list_config_general_settings(services(&state, &mut config), &client).await?
    };
    Ok(Json(fields))
}

// POST /config/field/update and /config/field/delete share the caller check, the config lock and the row edit
async fn respond_write(
    state: &HttpState,
    caller: Option<&Extension<AuthContext>>,
    request: FieldWrite,
) -> Response {
    let client = match caller_of(caller) {
        Ok(client) => client,
        Err(error) => return error.into_response(),
    };
    let row = {
        let mut config = state.fields.config.lock().await;
        match request {
            FieldWrite::Update(update) => {
                update_config_general_settings(services(state, &mut config), &client, &update).await
            }
            FieldWrite::Delete(delete) => {
                delete_config_general_settings(services(state, &mut config), &client, &delete).await
            }
        }
    };
    respond(row)
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

fn respond<T>(result: Result<T, ConfigFieldError>) -> Response
where
    T: Serialize,
{
    match result {
        Ok(body) => Json(body).into_response(),
        Err(error) => error.into_response(),
    }
}
