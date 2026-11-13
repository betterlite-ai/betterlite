#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::nodes::{
    ClusterNodeInput, StartupNodesInput,
};
use betterlite_redis::redis_connection_settings::model::scalars::IntOrText;
use rstest::rstest;
use secrecy::ExposeSecret;

use super::support::{
    FakeRedis, dependencies, environment, redis_settings, settings, test_backend,
};
use crate::configuration::cache::{Backend, Cache};
use crate::configuration::callback_registration::CacheCallbacks;
use crate::configuration::settings::{
    CacheSettings, DEFAULT_REDIS_MAJOR_VERSION, DEFAULT_REDIS_TTL_SECONDS,
};
use crate::types::cache_type::{CacheMode, LiteLlmCacheType};
use crate::types::call_type::{CachingSupportedCallType, SUPPORTED_CALL_TYPES};
use crate::types::semantic::SemanticCacheScope;

#[tokio::test]
async fn local_type_builds_a_memory_backend() {
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(
        settings(LiteLlmCacheType::Local),
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("a local cache");
    assert!(matches!(cache.backend, Backend::Memory(_)));
    assert_eq!(cache.cache_type, LiteLlmCacheType::Local);
}

#[tokio::test]
async fn redis_type_builds_a_single_node_backend() {
    let server = FakeRedis::start().await;
    let mut redis_config = redis_settings(&server);
    redis_config.namespace = Some("models".to_owned());
    redis_config.redis_default_ttl = 42;
    redis_config.redis_flush_size = Some(7);
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(
        redis_config,
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("a redis cache");
    let Backend::Redis(redis_cache) = &cache.backend else {
        panic!("expected a redis backend");
    };
    assert_eq!(redis_cache.transport.family, RedisCacheFamily::Standalone);
    assert_eq!(redis_cache.namespace.as_deref(), Some("models"));
    assert_eq!(redis_cache.default_ttl, 42);
    assert_eq!(redis_cache.redis_major_version, DEFAULT_REDIS_MAJOR_VERSION);
    assert_eq!(
        redis_cache.transport.settings.host.as_deref(),
        Some(server.host().as_str())
    );
    assert_eq!(
        redis_cache.transport.settings.port,
        Some(i64::from(server.port()))
    );
    assert_eq!(cache.namespace.as_deref(), Some("models"));
    assert_eq!(cache.redis_flush_size, Some(7));
}

#[tokio::test]
async fn disk_type_builds_a_disk_backend() {
    let directory = tempfile::tempdir().expect("a temp dir");
    let mut disk = settings(LiteLlmCacheType::Disk);
    disk.disk_cache_dir = Some(directory.path().to_owned());
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(disk, &mut callbacks, &environment(), &dependencies())
        .await
        .expect("a disk cache");
    assert!(matches!(cache.backend, Backend::Disk(_)));
    assert_eq!(cache.cache_type, LiteLlmCacheType::Disk);
}

#[tokio::test]
async fn an_injected_backend_is_used_as_given() {
    let (injected, _disconnects) = test_backend();
    let mut injected_settings = settings(LiteLlmCacheType::Local);
    injected_settings.backend = Some(Arc::clone(&injected));
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(
        injected_settings,
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("an injected cache");
    let Backend::Injected(backend) = &cache.backend else {
        panic!("expected the injected backend");
    };
    assert!(Arc::ptr_eq(backend, &injected));
}

#[tokio::test]
async fn an_injected_backend_skips_the_type_branches() {
    let (injected, _disconnects) = test_backend();
    let mut injected_settings = settings(LiteLlmCacheType::S3);
    injected_settings.backend = Some(injected);
    injected_settings.ttl = Some(12.0_f64);
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(
        injected_settings,
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("an injected cache");
    assert_eq!(cache.cache_type, LiteLlmCacheType::S3);
    assert_eq!(cache.ttl, Some(12.0_f64));
    assert!(matches!(cache.backend, Backend::Injected(_)));
}

#[rstest]
#[case::redis_semantic(
    LiteLlmCacheType::RedisSemantic,
    "cache type 'redis-semantic' is owned by B4382"
)]
#[case::valkey_semantic(
    LiteLlmCacheType::ValkeySemantic,
    "cache type 'valkey-semantic' is owned by B8296"
)]
#[case::qdrant_semantic(
    LiteLlmCacheType::QdrantSemantic,
    "cache type 'qdrant-semantic' is owned by B4383"
)]
#[case::s3(LiteLlmCacheType::S3, "cache type 's3' is owned by B4350")]
#[case::azure_blob(
    LiteLlmCacheType::AzureBlob,
    "cache type 'azure-blob' is owned by B4351"
)]
#[case::gcs(LiteLlmCacheType::Gcs, "cache type 'gcs' is not yet implemented")]
#[tokio::test]
async fn unsupported_types_name_the_task_that_owns_them(
    #[case] cache_type: LiteLlmCacheType,
    #[case] expected: &str,
) {
    let mut callbacks = CacheCallbacks::default();
    let error = Cache::open(
        settings(cache_type),
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .err()
    .expect("an unsupported cache type");
    assert_eq!(error.to_string(), expected);
    assert_eq!(callbacks, CacheCallbacks::default());
}

#[tokio::test]
async fn a_startup_node_list_selects_the_unsupported_cluster_mode() {
    let mut cluster = settings(LiteLlmCacheType::Redis);
    cluster.redis.startup_nodes = Some(StartupNodesInput::Value(vec![ClusterNodeInput {
        host: "127.0.0.1".to_owned(),
        port: IntOrText::Value(7000),
    }]));
    let mut callbacks = CacheCallbacks::default();
    let error = Cache::open(cluster, &mut callbacks, &environment(), &dependencies())
        .await
        .err()
        .expect("an unsupported cluster cache");
    assert_eq!(
        error.to_string(),
        "cache type 'redis' with cluster mode is owned by B5640"
    );
}

#[tokio::test]
async fn cluster_nodes_in_the_environment_select_the_unsupported_cluster_mode() {
    let cluster_environment = BTreeMap::from([(
        "REDIS_CLUSTER_NODES".to_owned(),
        r#"[{"host": "127.0.0.1", "port": 7000}]"#.to_owned(),
    )]);
    let mut callbacks = CacheCallbacks::default();
    let error = Cache::open(
        settings(LiteLlmCacheType::Redis),
        &mut callbacks,
        &cluster_environment,
        &dependencies(),
    )
    .await
    .err()
    .expect("an unsupported cluster cache");
    assert_eq!(
        error.to_string(),
        "cache type 'redis' with cluster mode is owned by B5640"
    );
}

#[tokio::test]
async fn an_empty_startup_node_list_still_reads_the_environment() {
    let cluster_environment = BTreeMap::from([(
        "REDIS_CLUSTER_NODES".to_owned(),
        r#"[{"host": "127.0.0.1", "port": 7000}]"#.to_owned(),
    )]);
    let mut empty_list = settings(LiteLlmCacheType::Redis);
    empty_list.redis.startup_nodes = Some(StartupNodesInput::Value(Vec::new()));
    let mut callbacks = CacheCallbacks::default();
    let error = Cache::open(
        empty_list,
        &mut callbacks,
        &cluster_environment,
        &dependencies(),
    )
    .await
    .err()
    .expect("an unsupported cluster cache");
    assert_eq!(
        error.to_string(),
        "cache type 'redis' with cluster mode is owned by B5640"
    );
}

#[tokio::test]
async fn an_empty_startup_node_list_falls_through_to_the_url_from_the_environment() {
    let server = FakeRedis::start().await;
    let redis_url = format!("redis://{}:{}", server.host(), server.port());
    let redis_environment = BTreeMap::from([("REDIS_URL".to_owned(), redis_url.clone())]);
    let mut empty_list = settings(LiteLlmCacheType::Redis);
    empty_list.redis.startup_nodes = Some(StartupNodesInput::Value(Vec::new()));
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(
        empty_list,
        &mut callbacks,
        &redis_environment,
        &dependencies(),
    )
    .await
    .expect("a single node cache");
    let Backend::Redis(redis) = &cache.backend else {
        panic!("expected a redis backend");
    };
    assert_eq!(
        redis
            .transport
            .settings
            .url
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(redis_url.as_str())
    );
    assert_eq!(cache.ping().await.expect("a pong"), Some(true));
}

#[tokio::test]
async fn an_empty_cluster_nodes_variable_counts_as_unset() {
    let server = FakeRedis::start().await;
    let redis_environment = BTreeMap::from([
        ("REDIS_CLUSTER_NODES".to_owned(), String::new()),
        ("REDIS_HOST".to_owned(), server.host()),
        ("REDIS_PORT".to_owned(), server.port().to_string()),
    ]);
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(
        settings(LiteLlmCacheType::Redis),
        &mut callbacks,
        &redis_environment,
        &dependencies(),
    )
    .await
    .expect("a single node cache");
    let Backend::Redis(redis) = &cache.backend else {
        panic!("expected a redis backend");
    };
    assert_eq!(
        redis.transport.settings.host.as_deref(),
        Some(server.host().as_str())
    );
}

#[tokio::test]
async fn the_redis_backend_uses_the_configured_flush_size() {
    let server = FakeRedis::start().await;
    let mut redis_config = redis_settings(&server);
    redis_config.redis_flush_size = Some(2);
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(
        redis_config,
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("a redis cache");
    let Backend::Redis(redis_cache) = &cache.backend else {
        panic!("expected a redis backend");
    };
    redis_cache
        .values
        .batch_cache_write("alpha", "first")
        .await
        .expect("the first buffered write");
    assert!(server.commands_named("SET").is_empty());
    redis_cache
        .values
        .batch_cache_write("beta", "second")
        .await
        .expect("the flushing write");
    assert_eq!(server.commands_named("SET").len(), 2);
}

#[tokio::test]
async fn the_redis_settings_read_the_injected_environment() {
    let server = FakeRedis::start().await;
    let redis_environment = BTreeMap::from([
        ("REDIS_HOST".to_owned(), server.host()),
        ("REDIS_PORT".to_owned(), server.port().to_string()),
    ]);
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(
        settings(LiteLlmCacheType::Redis),
        &mut callbacks,
        &redis_environment,
        &dependencies(),
    )
    .await
    .expect("a redis cache");
    let Backend::Redis(redis) = &cache.backend else {
        panic!("expected a redis backend");
    };
    assert_eq!(
        redis.transport.settings.host.as_deref(),
        Some(server.host().as_str())
    );
    assert_eq!(
        redis.transport.settings.port,
        Some(i64::from(server.port()))
    );
}

#[tokio::test]
async fn default_in_memory_ttl_applies_only_to_a_local_cache() {
    let mut local = settings(LiteLlmCacheType::Local);
    local.ttl = Some(1.0_f64);
    local.default_in_memory_ttl = Some(5.0_f64);
    let mut callbacks = CacheCallbacks::default();
    let local_cache = Cache::open(local, &mut callbacks, &environment(), &dependencies())
        .await
        .expect("a local cache");
    assert_eq!(local_cache.ttl, Some(5.0_f64));

    let mut local_with_redis_ttl = settings(LiteLlmCacheType::Local);
    local_with_redis_ttl.ttl = Some(1.0_f64);
    local_with_redis_ttl.default_in_redis_ttl = Some(7.0_f64);
    let local_with_redis_ttl_cache = Cache::open(
        local_with_redis_ttl,
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("a local cache");
    assert_eq!(local_with_redis_ttl_cache.ttl, Some(1.0_f64));
}

#[tokio::test]
async fn default_in_redis_ttl_applies_only_to_a_redis_cache() {
    let server = FakeRedis::start().await;
    let mut redis = redis_settings(&server);
    redis.ttl = Some(1.0_f64);
    redis.default_in_redis_ttl = Some(7.0_f64);
    let mut callbacks = CacheCallbacks::default();
    let redis_default_cache = Cache::open(redis, &mut callbacks, &environment(), &dependencies())
        .await
        .expect("a redis cache");
    assert_eq!(redis_default_cache.ttl, Some(7.0_f64));

    let mut redis_with_memory_ttl = redis_settings(&server);
    redis_with_memory_ttl.ttl = Some(1.0_f64);
    redis_with_memory_ttl.default_in_memory_ttl = Some(5.0_f64);
    let redis_with_memory_ttl_cache = Cache::open(
        redis_with_memory_ttl,
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("a redis cache");
    assert_eq!(redis_with_memory_ttl_cache.ttl, Some(1.0_f64));
}

#[rstest]
#[case::disk(LiteLlmCacheType::Disk)]
#[case::s3(LiteLlmCacheType::S3)]
#[case::qdrant_semantic(LiteLlmCacheType::QdrantSemantic)]
#[case::azure_blob(LiteLlmCacheType::AzureBlob)]
#[case::gcs(LiteLlmCacheType::Gcs)]
#[tokio::test]
async fn other_types_keep_the_configured_ttl(#[case] cache_type: LiteLlmCacheType) {
    let (injected, _disconnects) = test_backend();
    let mut other = settings(cache_type);
    other.ttl = Some(1.0_f64);
    other.default_in_memory_ttl = Some(5.0_f64);
    other.default_in_redis_ttl = Some(7.0_f64);
    other.backend = Some(injected);
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(other, &mut callbacks, &environment(), &dependencies())
        .await
        .expect("an injected cache");
    assert_eq!(cache.ttl, Some(1.0_f64));
}

#[rstest]
#[case::redis_semantic(LiteLlmCacheType::RedisSemantic)]
#[case::valkey_semantic(LiteLlmCacheType::ValkeySemantic)]
#[tokio::test]
async fn an_injected_semantic_cache_keeps_the_redis_ttl_rule(#[case] cache_type: LiteLlmCacheType) {
    let (injected, _disconnects) = test_backend();
    let mut semantic = settings(cache_type);
    semantic.ttl = Some(1.0_f64);
    semantic.default_in_redis_ttl = Some(9.0_f64);
    semantic.backend = Some(injected);
    let mut callbacks = CacheCallbacks::default();
    let semantic_cache = Cache::open(semantic, &mut callbacks, &environment(), &dependencies())
        .await
        .expect("an injected semantic cache");
    assert_eq!(semantic_cache.ttl, Some(9.0_f64));
}

#[tokio::test]
async fn the_configured_attributes_are_stored() {
    let mut configured = settings(LiteLlmCacheType::Local);
    configured.mode = CacheMode::DefaultOff;
    configured.namespace = Some("team".to_owned());
    configured.ttl = Some(30.0_f64);
    configured.supported_call_types = vec![CachingSupportedCallType::Completion];
    configured.semantic_cache_scope = SemanticCacheScope::EndUser;
    configured.redis_flush_size = Some(9);
    let mut callbacks = CacheCallbacks::default();
    let configured_cache = Cache::open(configured, &mut callbacks, &environment(), &dependencies())
        .await
        .expect("a local cache");
    assert_eq!(configured_cache.cache_type, LiteLlmCacheType::Local);
    assert_eq!(configured_cache.mode, CacheMode::DefaultOff);
    assert_eq!(configured_cache.namespace.as_deref(), Some("team"));
    assert_eq!(configured_cache.ttl, Some(30.0_f64));
    assert_eq!(
        configured_cache.supported_call_types,
        vec![CachingSupportedCallType::Completion]
    );
    assert_eq!(
        configured_cache.semantic_cache_scope,
        SemanticCacheScope::EndUser
    );
    assert_eq!(configured_cache.redis_flush_size, Some(9));
}

#[test]
fn cache_settings_default_to_litellm_values() {
    let defaults = CacheSettings::default();
    assert_eq!(defaults.cache_type, LiteLlmCacheType::Local);
    assert_eq!(defaults.mode, CacheMode::DefaultOn);
    assert_eq!(defaults.semantic_cache_scope, SemanticCacheScope::Key);
    assert_eq!(defaults.supported_call_types, SUPPORTED_CALL_TYPES.to_vec());
    assert_eq!(defaults.redis_default_ttl, DEFAULT_REDIS_TTL_SECONDS);
    assert_eq!(defaults.redis_default_ttl, 60);
    assert_eq!(
        defaults.redis_default_major_version,
        DEFAULT_REDIS_MAJOR_VERSION
    );
    assert!(defaults.backend.is_none());
}
