use std::error::Error as StdError;

use betterlite_cache::configuration::backend_construction::BackendConstructionError;
use betterlite_cache::configuration::settings::CacheSettings;
use betterlite_cache::configuration::settings::DEFAULT_REDIS_MAJOR_VERSION;
use betterlite_cache::configuration::settings::DEFAULT_REDIS_TTL_SECONDS;
use betterlite_cache::types::cache_type::CacheMode;
use betterlite_cache::types::cache_type::LiteLlmCacheType;
use betterlite_cache::types::call_type::SUPPORTED_CALL_TYPES;
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use serde_json::{Map, Value};
use thiserror::Error;

const CACHE_STARTUP_NODES_OPTION: &str = "redis_startup_nodes";
const STARTUP_NODES_OPTION: &str = "startup_nodes";
const URL_OPTION: &str = "url";

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

// The serde error quotes the offending value, so the message names only the option's path and the source stays unformatted.
#[derive(Debug, Error)]
pub(super) enum ConnectionTestError {
    #[error("the redis option \"{}\" has the wrong type or value", .source.path())]
    Options {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("the cache could not be constructed")]
    Construction(#[source] BackendConstructionError),
    #[error("the temporary cache is not a redis cache")]
    NotRedis,
}

// Python binds Cache's named options and hands the rest to redis-py as keyword arguments; the map is split the same way.
pub(super) fn cache_settings_from_map(
    settings: &Map<String, Value>,
) -> Result<CacheSettings, ConnectionTestError> {
    let mut options = settings.clone();
    if let Some(startup_nodes) = options.remove(CACHE_STARTUP_NODES_OPTION) {
        options.insert(STARTUP_NODES_OPTION.to_owned(), startup_nodes);
    }
    options.retain(|name, _value| !CACHE_ONLY_OPTIONS.split(' ').any(|known| known == name));
    // redis-py treats a null kwarg as unset, while the passthrough map has no null: only an explicit url null carries meaning.
    options.retain(|_name, value| !value.is_null());
    if settings.get(URL_OPTION).is_some_and(Value::is_null) {
        options.insert(URL_OPTION.to_owned(), Value::Null);
    }
    let redis: RedisOverrides = serde_path_to_error::deserialize(Value::Object(options))
        .map_err(|source| ConnectionTestError::Options { source })?;
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

// LiteLLM reports the exception raised inside the cache; the outermost betterlite error only says the cache could not be created.
pub(super) fn deepest_source(source: &(dyn StdError + 'static)) -> String {
    let mut text = source.to_string();
    let mut cause = source.source();
    while let Some(inner) = cause {
        text = inner.to_string();
        cause = inner.source();
    }
    text
}
