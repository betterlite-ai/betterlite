use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Extension;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_redis::redis_connection_settings::environment::EnvironmentLookup;
use betterlite_secrets::secret_codec::CipherSettings;
use betterlite_secrets::secret_codec::OsNonces;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use serde::Serialize;
use serde_json::{Map, Value};

use super::fields::catalog;
use super::fields::{CacheSettingsField, REDIS_TYPE_DESCRIPTIONS, RedisTypeDescriptions};
use super::stored::{CacheConfigStore, StoredSettingsError, stored_settings};
use super::view::{effective_values, fill_field_values};
use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::router::unauthenticated;

#[derive(Clone)]
pub struct CacheSettingsReadState {
    store: Option<Arc<dyn CacheConfigStore>>,
    cipher: Arc<DbCipher>,
    environment: Arc<dyn EnvironmentLookup + Send + Sync>,
}

impl CacheSettingsReadState {
    #[must_use]
    pub fn new(
        store: Option<Arc<dyn CacheConfigStore>>,
        cipher: CipherSettings,
        environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    ) -> Self {
        Self {
            store,
            cipher: stored_cipher(cipher),
            environment,
        }
    }
}

pub fn router(state: CacheSettingsReadState) -> Router {
    Router::new()
        .route("/cache/settings", get(get_cache_settings))
        .with_state(state)
}

#[derive(Serialize)]
struct CacheSettingsResponse {
    fields: Vec<CacheSettingsField>,
    current_values: Map<String, Value>,
    redis_type_descriptions: RedisTypeDescriptions,
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

async fn get_cache_settings(
    State(state): State<CacheSettingsReadState>,
    caller: Option<Extension<AuthContext>>,
) -> Response {
    // The auth layer refuses a missing or non-admin key before this route; the guard keeps it safe on its own.
    if caller.is_none() {
        return unauthenticated();
    }
    let stored = match state.store.as_deref() {
        Some(store) => match stored_settings(store, &state.cipher).await {
            Ok(stored) => stored,
            Err(error) => return fetch_failure(&error),
        },
        None => Map::new(),
    };
    let current_values = effective_values(&stored, state.environment.as_ref());
    let mut fields = catalog();
    fill_field_values(&mut fields, &current_values);
    (
        StatusCode::OK,
        Json(CacheSettingsResponse {
            fields,
            current_values,
            redis_type_descriptions: REDIS_TYPE_DESCRIPTIONS,
        }),
    )
        .into_response()
}

fn fetch_failure(error: &StoredSettingsError) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(DetailBody {
            detail: format!("Error fetching cache settings: {error}"),
        }),
    )
        .into_response()
}

#[must_use]
pub(crate) fn stored_cipher(settings: CipherSettings) -> Arc<DbCipher> {
    Arc::new(DbCipher {
        settings,
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    })
}

mod tests;
