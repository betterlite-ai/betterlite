use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::Extension;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_db::stored_cipher::DbCipher;
use serde::Serialize;

use super::audit::CacheSettingsAudit;
use super::manager::CacheReinit;
use super::manager::CacheSettingsManager;
use super::store::CacheConfigRepository;
use crate::audit_events::callbacks::AuditTaskSpawner;
use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::router::unauthenticated;

use self::pipeline::UpdateError;
use self::pipeline::update;

mod pipeline;
mod tests;
mod validation;

const ADMIN_REQUIRED: &str = "Admin access required to update cache settings";
const CHANGED_BY_HEADER: &str = "litellm-changed-by";
const NO_DATABASE: &str = "Database not connected. Please connect a database.";
const STORE_MODEL_IN_DB_REQUIRED: &str =
    "Set `'STORE_MODEL_IN_DB='True'` in your env to enable this feature.";

// LiteLLM reads the live store_model_in_db global on every request, so the route asks a shared value here too.
pub trait StoreModelInDbSettings: Send + Sync {
    fn store_model_in_db(&self) -> bool;
}

#[derive(Clone)]
pub struct CacheSettingsUpdateState {
    pub repository: Option<Arc<dyn CacheConfigRepository>>,
    pub cipher: Arc<DbCipher>,
    pub manager: Arc<CacheSettingsManager>,
    pub reinit: Arc<dyn CacheReinit>,
    pub audit: Arc<dyn CacheSettingsAudit>,
    pub spawner: Arc<dyn AuditTaskSpawner>,
    pub store_model_in_db: Arc<dyn StoreModelInDbSettings>,
}

pub fn router(state: CacheSettingsUpdateState) -> Router {
    Router::new()
        .route("/cache/settings", post(update_cache_settings))
        .with_state(state)
}

#[derive(Serialize)]
struct ErrorDetail<'message> {
    error: &'message str,
}

#[derive(Serialize)]
struct ObjectDetail<'message> {
    detail: ErrorDetail<'message>,
}

#[derive(Serialize)]
struct TextDetail {
    detail: String,
}

async fn update_cache_settings(
    State(state): State<CacheSettingsUpdateState>,
    caller: Option<Extension<AuthContext>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    if !context.principal.roles.contains(&Role::PlatformAdmin) {
        return forbidden();
    }
    let settings = match validation::decode_request(&body) {
        Ok(settings) => settings,
        Err(error) => return error.into_response(),
    };
    let Some(repository) = state.repository.as_deref() else {
        return configuration_failure(NO_DATABASE);
    };
    if !state.store_model_in_db.store_model_in_db() {
        return configuration_failure(STORE_MODEL_IN_DB_REQUIRED);
    }
    let changed_by = headers.get(CHANGED_BY_HEADER).map(|value| {
        value
            .as_bytes()
            .iter()
            .copied()
            .map(char::from)
            .collect::<String>()
    });
    match update(
        &state,
        repository,
        &settings,
        changed_by.as_deref(),
        &context.principal,
    )
    .await
    {
        Ok(response) => response,
        Err(error) => failure(&error),
    }
}

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(TextDetail {
            detail: ADMIN_REQUIRED.to_owned(),
        }),
    )
        .into_response()
}

fn configuration_failure(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ObjectDetail {
            detail: ErrorDetail { error: message },
        }),
    )
        .into_response()
}

fn failure(error: &UpdateError) -> Response {
    let detail = format!("Error updating cache settings: {error}");
    tracing::error!("{detail}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(TextDetail { detail }),
    )
        .into_response()
}
