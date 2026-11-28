use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::Extension;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_cache::configuration::cache::Backend;
use betterlite_cache::configuration::cache::Cache;
use betterlite_cache::configuration::callback_registration::CacheCallbacks;
use betterlite_cache::configuration::settings::CacheDependencies;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_error::envelope::ErrorObject;
use betterlite_error::envelope::ErrorResponse;
use betterlite_error::envelope::WireField;
use betterlite_redis::redis_cache_admin::connection_test::ConnectionStatus;
use betterlite_redis::redis_cache_admin::connection_test::ConnectionTestResult;
use betterlite_redis::redis_client_reuse::breaker::BreakerAdmission;
use betterlite_redis::redis_client_reuse::breaker::BreakerOpen;
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_live_client::LiveClientFactory;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::auth::auth_identity::AuthContext;
use crate::cache_settings::merge::merge_over_saved;
use crate::cache_settings::precedence::resolve_cache_url_precedence;
use crate::cache_settings::redact::redact_credentials;

use super::saved::SavedCacheConfig;
use super::saved::saved_settings;

use self::bridge::ConnectionTestError;
use self::bridge::cache_settings_from_map;
use self::bridge::deepest_source;
use self::validation::decode_request;

mod bridge;
mod tests;
mod validation;

const ADMIN_REQUIRED: &str = "Admin access required to test cache settings";
const NOT_SUPPORTED_MESSAGE: &str = "Only Redis cache type is currently supported for testing";
const REDIS_CACHE_TYPE: &str = "redis";
const UNAUTHENTICATED: &str = "the API key is not valid";

#[derive(Clone)]
pub struct CacheTestState {
    pub saved: Option<Arc<dyn SavedCacheConfig>>,
    pub cipher: Arc<DbCipher>,
    pub redis_factory: Arc<LiveClientFactory>,
    pub environment: BTreeMap<String, String>,
}

pub fn router(state: CacheTestState) -> Router {
    Router::new()
        .route("/cache/settings/test", post(test_cache_connection))
        .with_state(state)
}

#[derive(Serialize)]
struct CacheTestResponse {
    status: ConnectionStatus,
    message: String,
    error: Option<String>,
}

#[derive(Serialize)]
struct AdminDetail {
    detail: &'static str,
}

async fn test_cache_connection(
    State(state): State<CacheTestState>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    if !context.principal.roles.contains(&Role::PlatformAdmin) {
        return forbidden();
    }
    let request = match decode_request(&body) {
        Ok(request) => request,
        Err(error) => return error.into_response(),
    };
    let settings = resolved_settings(&request.cache_settings, &state).await;
    let response = connection_answer(&settings, &state).await;
    (StatusCode::OK, Json(response)).into_response()
}

// The saved row is read first, the request overrides it, and a url wins over the discrete target fields last.
async fn resolved_settings(
    incoming: &Map<String, Value>,
    state: &CacheTestState,
) -> Map<String, Value> {
    let saved = saved_settings(state.saved.as_deref(), &state.cipher).await;
    let resolved = resolve_cache_url_precedence(&merge_over_saved(incoming, &saved));
    // tracing's field expansion imports a Value trait, so the redacted map is built before the macro names Value.
    let redacted = Value::Object(redact_credentials(&resolved));
    tracing::debug!(
        cache_settings = %redacted,
        "Testing cache connection with settings"
    );
    resolved
}

async fn connection_answer(
    settings: &Map<String, Value>,
    state: &CacheTestState,
) -> CacheTestResponse {
    if !matches!(settings.get("type"), Some(Value::String(kind)) if kind == REDIS_CACHE_TYPE) {
        return CacheTestResponse {
            status: ConnectionStatus::Failed,
            message: NOT_SUPPORTED_MESSAGE.to_owned(),
            error: None,
        };
    }
    match connection_test(settings, state).await {
        Ok(result) => CacheTestResponse {
            status: result.status,
            message: result.message,
            error: result.error,
        },
        Err(error) => {
            let error_text = match &error {
                ConnectionTestError::Construction(source) => deepest_source(source),
                ConnectionTestError::Options { .. } | ConnectionTestError::NotRedis => {
                    error.to_string()
                }
            };
            CacheTestResponse {
                status: ConnectionStatus::Failed,
                message: format!("Cache connection test failed: {error_text}"),
                error: Some(error_text),
            }
        }
    }
}

async fn connection_test(
    settings: &Map<String, Value>,
    state: &CacheTestState,
) -> Result<ConnectionTestResult, ConnectionTestError> {
    let cache_settings = cache_settings_from_map(settings)?;
    let mut callbacks = CacheCallbacks::default();
    let dependencies = CacheDependencies {
        clock: Arc::new(|| 0.0_f64),
        redis_factory: Arc::clone(&state.redis_factory),
        redis_store: Arc::new(RedisClientStore::new()),
        redis_hooks: Arc::new(SilentHooks),
        redis_gate: Arc::new(ClosedGate),
    };
    let cache = Cache::open(
        cache_settings,
        &mut callbacks,
        &state.environment,
        &dependencies,
    )
    .await
    .map_err(ConnectionTestError::Construction)?;
    match &cache.backend {
        Backend::Redis(redis) => Ok(redis.admin.test_connection().await),
        Backend::Memory(_) | Backend::Disk(_) | Backend::Injected(_) => {
            Err(ConnectionTestError::NotRedis)
        }
    }
}

// LiteLLM's auth_error carries param "None" rather than null, which WireField::MISSING serializes
fn unauthenticated() -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message: UNAUTHENTICATED.to_owned(),
            error_type: WireField::Text("auth_error".to_owned()),
            param: WireField::MISSING,
            code: WireField::Text("401".to_owned()),
        },
    };
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
}

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(AdminDetail {
            detail: ADMIN_REQUIRED,
        }),
    )
        .into_response()
}

struct SilentHooks;

impl ServiceHooks for SilentHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn StdError + Send + Sync)) {}
}

struct ClosedGate;

impl BreakerGate for ClosedGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0_u64))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn StdError + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn StdError + Send + Sync)) {}
}
