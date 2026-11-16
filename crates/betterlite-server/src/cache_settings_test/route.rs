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
use betterlite_cache::configuration::backend_construction::BackendConstructionError;
use betterlite_cache::configuration::cache::Backend;
use betterlite_cache::configuration::cache::Cache;
use betterlite_cache::configuration::callback_registration::CacheCallbacks;
use betterlite_cache::configuration::settings::CacheDependencies;
use betterlite_cache::configuration::settings::CacheSettings;
use betterlite_cache::configuration::settings::DEFAULT_REDIS_MAJOR_VERSION;
use betterlite_cache::configuration::settings::DEFAULT_REDIS_TTL_SECONDS;
use betterlite_cache::types::cache_type::CacheMode;
use betterlite_cache::types::cache_type::LiteLlmCacheType;
use betterlite_cache::types::call_type::SUPPORTED_CALL_TYPES;
use betterlite_cache::types::semantic::SemanticCacheScope;
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
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;

use crate::auth::auth_identity::AuthContext;
use crate::cache_settings::merge::merge_over_saved;
use crate::cache_settings::precedence::resolve_cache_url_precedence;
use crate::cache_settings::redact::redact_credentials;

use super::saved::SavedCacheConfig;
use super::saved::saved_settings;

mod tests;

const ADMIN_REQUIRED: &str = "Admin access required to test cache settings";
const MASTER_KEY_SUBJECT: &str = "master_key";
const NOT_SUPPORTED_MESSAGE: &str = "Only Redis cache type is currently supported for testing";
const REDIS_CACHE_TYPE: &str = "redis";
const UNAUTHENTICATED: &str = "the API key is not valid";
const CACHE_STARTUP_NODES_OPTION: &str = "redis_startup_nodes";
const STARTUP_NODES_OPTION: &str = "startup_nodes";
const URL_OPTION: &str = "url";
const FIELD_REQUIRED: &str = "Field required";
const JSON_DECODE_ERROR: &str = "JSON decode error";
const DICT_TYPE_MESSAGE: &str = "Input should be a valid dictionary";
const MODEL_ATTRIBUTES_MESSAGE: &str =
    "Input should be a valid dictionary or object to extract fields from";

// Cache.__init__ binds these by name before its **kwargs reach RedisCache and redis-py, so none of them becomes a client option.
const CACHE_ONLY_OPTIONS: &str = "type mode namespace ttl default_in_memory_ttl default_in_redis_ttl \
    similarity_threshold supported_call_types azure_account_url azure_blob_container s3_bucket_name \
    s3_region_name s3_api_version s3_use_ssl s3_verify s3_endpoint_url s3_aws_access_key_id \
    s3_aws_secret_access_key s3_aws_session_token s3_config s3_path gcs_bucket_name \
    gcs_path_service_account gcs_path redis_semantic_cache_embedding_model \
    redis_semantic_cache_index_name valkey_semantic_cache_embedding_model \
    valkey_semantic_cache_index_name redis_flush_size disk_cache_dir qdrant_api_base \
    qdrant_api_key qdrant_collection_name qdrant_quantization_config \
    qdrant_semantic_cache_embedding_model qdrant_semantic_cache_vector_size \
    semantic_cache_embedding_max_input_tokens semantic_cache_embedding_timeout semantic_cache_scope \
    gcp_service_account gcp_ssl_ca_certs _backend";

fn is_cache_only(name: &str) -> bool {
    CACHE_ONLY_OPTIONS.split(' ').any(|known| known == name)
}

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

struct CacheTestRequest {
    cache_settings: Map<String, Value>,
}

#[derive(Serialize)]
struct ValidationBody {
    detail: Vec<ValidationIssue>,
}

#[derive(Serialize)]
struct ValidationIssue {
    #[serde(rename = "type")]
    kind: &'static str,
    loc: &'static [&'static str],
    msg: &'static str,
}

#[derive(Serialize)]
struct AdminDetail {
    detail: &'static str,
}

// FastAPI answers a bad body with one issue; a null body counts as absent and a non-object body never reaches the model.
enum BodyError {
    MissingBody,
    BodyNotAnObject,
    MissingCacheSettings,
    CacheSettingsNotAnObject,
    InvalidJson,
}

#[derive(Debug, Error)]
#[error("the cache settings do not name valid redis options: {source}")]
struct CacheOptionsError {
    #[source]
    source: serde_path_to_error::Error<serde_json::Error>,
}

enum ConnectionTestError {
    Options(CacheOptionsError),
    Construction(BackendConstructionError),
    NotRedis,
}

async fn test_cache_connection(
    State(state): State<CacheTestState>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    if !is_proxy_admin(&context) {
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
    tracing::debug!(
        cache_settings = %Value::Object(redact_credentials(&resolved)),
        "Testing cache connection with settings"
    );
    resolved
}

async fn connection_answer(
    settings: &Map<String, Value>,
    state: &CacheTestState,
) -> CacheTestResponse {
    if !is_redis(settings) {
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
        Err(error) => failed(error.text()),
    }
}

fn is_proxy_admin(caller: &AuthContext) -> bool {
    caller.principal.subject == MASTER_KEY_SUBJECT
        || caller.principal.roles.contains(&Role::PlatformAdmin)
}

fn is_redis(settings: &Map<String, Value>) -> bool {
    matches!(settings.get("type"), Some(Value::String(kind)) if kind == REDIS_CACHE_TYPE)
}

async fn connection_test(
    settings: &Map<String, Value>,
    state: &CacheTestState,
) -> Result<ConnectionTestResult, ConnectionTestError> {
    let cache_settings = cache_settings_from_map(settings).map_err(ConnectionTestError::Options)?;
    let mut callbacks = CacheCallbacks::default();
    let dependencies = cache_dependencies(state);
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

// Python binds Cache's named options and hands the rest to redis-py as keyword arguments; the map is split the same way.
fn cache_settings_from_map(
    settings: &Map<String, Value>,
) -> Result<CacheSettings, CacheOptionsError> {
    let mut options = settings.clone();
    if let Some(startup_nodes) = options.remove(CACHE_STARTUP_NODES_OPTION) {
        options.insert(STARTUP_NODES_OPTION.to_owned(), startup_nodes);
    }
    options.retain(|name, _value| !is_cache_only(name));
    // redis-py treats a null kwarg as unset, while the passthrough map has no null: only an explicit url null carries meaning.
    options.retain(|_name, value| !value.is_null());
    if settings.get(URL_OPTION).is_some_and(Value::is_null) {
        options.insert(URL_OPTION.to_owned(), Value::Null);
    }
    let redis: RedisOverrides = serde_path_to_error::deserialize(Value::Object(options))
        .map_err(|source| CacheOptionsError { source })?;
    Ok(CacheSettings {
        cache_type: LiteLlmCacheType::Redis,
        mode: CacheMode::DefaultOn,
        namespace: None,
        ttl: None,
        default_in_memory_ttl: None,
        default_in_redis_ttl: None,
        supported_call_types: SUPPORTED_CALL_TYPES.to_vec(),
        semantic_cache_scope: SemanticCacheScope::Key,
        redis,
        redis_flush_size: None,
        redis_default_ttl: DEFAULT_REDIS_TTL_SECONDS,
        redis_default_major_version: DEFAULT_REDIS_MAJOR_VERSION,
        disk_cache_dir: None,
        backend: None,
    })
}

fn cache_dependencies(state: &CacheTestState) -> CacheDependencies {
    CacheDependencies {
        clock: Arc::new(|| 0.0),
        redis_factory: Arc::clone(&state.redis_factory),
        redis_store: Arc::new(RedisClientStore::new()),
        redis_hooks: Arc::new(SilentHooks),
        redis_gate: Arc::new(ClosedGate),
    }
}

fn decode_request(body: &[u8]) -> Result<CacheTestRequest, BodyError> {
    if body.is_empty() {
        return Err(BodyError::MissingBody);
    }
    match serde_json::from_slice(body).map_err(|_error| BodyError::InvalidJson)? {
        Value::Object(fields) => cache_settings_from_body(fields),
        Value::Null => Err(BodyError::MissingBody),
        Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Err(BodyError::BodyNotAnObject)
        }
    }
}

fn cache_settings_from_body(mut fields: Map<String, Value>) -> Result<CacheTestRequest, BodyError> {
    match fields.remove("cache_settings") {
        Some(Value::Object(cache_settings)) => Ok(CacheTestRequest { cache_settings }),
        Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_),
        ) => Err(BodyError::CacheSettingsNotAnObject),
        None => Err(BodyError::MissingCacheSettings),
    }
}

impl BodyError {
    const fn issue(&self) -> ValidationIssue {
        match self {
            Self::MissingBody => ValidationIssue {
                kind: "missing",
                loc: &["body"],
                msg: FIELD_REQUIRED,
            },
            Self::BodyNotAnObject => ValidationIssue {
                kind: "model_attributes_type",
                loc: &["body"],
                msg: MODEL_ATTRIBUTES_MESSAGE,
            },
            Self::MissingCacheSettings => ValidationIssue {
                kind: "missing",
                loc: &["body", "cache_settings"],
                msg: FIELD_REQUIRED,
            },
            Self::CacheSettingsNotAnObject => ValidationIssue {
                kind: "dict_type",
                loc: &["body", "cache_settings"],
                msg: DICT_TYPE_MESSAGE,
            },
            Self::InvalidJson => ValidationIssue {
                kind: "json_invalid",
                loc: &["body"],
                msg: JSON_DECODE_ERROR,
            },
        }
    }
}

impl IntoResponse for BodyError {
    fn into_response(self) -> Response {
        let body = ValidationBody {
            detail: vec![self.issue()],
        };
        (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
    }
}

impl ConnectionTestError {
    fn text(&self) -> String {
        match self {
            Self::Options(error) => error.to_string(),
            Self::Construction(error) => deepest_source(error),
            Self::NotRedis => "the temporary cache is not a redis cache".to_owned(),
        }
    }
}

// LiteLLM reports the exception raised inside the cache; the outermost betterlite error only says the cache could not be created.
fn deepest_source(source: &(dyn StdError + 'static)) -> String {
    let mut text = source.to_string();
    let mut cause = source.source();
    while let Some(inner) = cause {
        text = inner.to_string();
        cause = inner.source();
    }
    text
}

fn failed(error_text: String) -> CacheTestResponse {
    CacheTestResponse {
        status: ConnectionStatus::Failed,
        message: format!("Cache connection test failed: {error_text}"),
        error: Some(error_text),
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
