#![cfg(test)]

use betterlite_cache::types::cache_type::CacheMode;
use betterlite_cache::types::cache_type::LiteLlmCacheType;
use betterlite_cache::types::call_type::SUPPORTED_CALL_TYPES;
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;

use super::CacheSettingsWire;

#[test]
fn an_absent_settings_key_takes_litellms_default() {
    let wire = CacheSettingsWire::default();
    assert_eq!(wire.cache_type, LiteLlmCacheType::Local);
    assert_eq!(wire.mode, CacheMode::DefaultOn);
    assert_eq!(wire.semantic_cache_scope, SemanticCacheScope::Key);
    assert!(!wire.include_provider_params);
    let settings = wire.settings();
    assert_eq!(settings.supported_call_types, SUPPORTED_CALL_TYPES.to_vec());
    assert!(settings.backend.is_none());
    assert_eq!(settings.ttl, None);
}

#[test]
fn an_explicit_settings_key_reaches_both_settings_types() {
    let wire = CacheSettingsWire {
        cache_type: LiteLlmCacheType::Redis,
        mode: CacheMode::DefaultOff,
        namespace: Some("ns".to_owned()),
        ttl: Some(30.0_f64),
        default_in_memory_ttl: Some(45.0_f64),
        supported_call_types: None,
        semantic_cache_scope: SemanticCacheScope::EndUser,
        include_provider_params: true,
        default_in_redis_ttl: None,
        redis: RedisOverrides::default(),
        redis_flush_size: None,
        disk_cache_dir: None,
    };
    let settings = wire.settings();
    assert_eq!(settings.ttl, Some(30.0_f64));
    assert_eq!(settings.default_in_memory_ttl, Some(45.0_f64));
    assert_eq!(settings.namespace.as_deref(), Some("ns"));
    assert_eq!(settings.cache_type, LiteLlmCacheType::Redis);
    let key = wire.key_settings();
    assert_eq!(key.namespace.as_deref(), Some("ns"));
    assert_eq!(key.mode, CacheMode::DefaultOff);
    assert_eq!(key.semantic_cache_scope, SemanticCacheScope::EndUser);
    assert!(key.include_provider_params);
    assert!(!key.is_semantic_cache());
}
