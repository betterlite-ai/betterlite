#![cfg(test)]

use betterlite_cache::configuration::cache::Cache;
use betterlite_cache::types::cache_type::CacheMode;
use betterlite_models::decisions::Json;
use betterlite_redact::structured::SensitiveDataMasker;
use secrecy::SecretString;
use serde_json::Number;

use super::super::redis_support::{
    local_cache, overrides, redis_cache_from, unreachable_overrides,
};
use super::super::support_tests::parsed_cache_params;
use super::{health_check_params, masked_cache_params};

fn with_ttl(mut cache: Cache) -> Cache {
    cache.ttl = Some(90.0_f64);
    cache.namespace = Some("team-ns".to_owned());
    cache.redis_flush_size = Some(25_usize);
    cache.mode = CacheMode::DefaultOff;
    cache
}

#[test]
fn the_masked_cache_params_list_the_cache_fields() {
    let params = parsed_cache_params(&masked_cache_params(&local_cache()));
    let Some(Json::List(call_types)) = params.get("supported_call_types") else {
        panic!("the supported call types are a list");
    };
    assert_eq!(call_types.len(), 14);
    assert_eq!(
        call_types.first(),
        Some(&Json::Text("completion".to_owned()))
    );
    assert_eq!(
        call_types.last(),
        Some(&Json::Text("aanthropic_messages".to_owned()))
    );
    assert_eq!(params.get("type"), Some(&Json::Text("local".to_owned())));
    assert_eq!(params.get("namespace"), Some(&Json::Null));
    assert_eq!(params.get("redis_flush_size"), Some(&Json::Null));
    assert_eq!(params.get("ttl"), Some(&Json::Null));
    assert_eq!(
        params.get("mode"),
        Some(&Json::Text("default_on".to_owned()))
    );
    assert_eq!(
        params.get("semantic_cache_scope"),
        Some(&Json::Text("key".to_owned()))
    );
}

#[test]
fn the_masked_cache_params_keep_ttl_namespace_and_flush_size() {
    let params = parsed_cache_params(&masked_cache_params(&with_ttl(local_cache())));
    assert_eq!(
        params.get("namespace"),
        Some(&Json::Text("team-ns".to_owned()))
    );
    assert_eq!(
        params.get("redis_flush_size"),
        Some(&Json::Number(Number::from(25_i64)))
    );
    assert_eq!(
        params.get("ttl"),
        Some(&Json::Number(Number::from_f64(90.0_f64).unwrap()))
    );
    assert_eq!(
        params.get("mode"),
        Some(&Json::Text("default_off".to_owned()))
    );
}

#[test]
fn a_local_cache_has_empty_health_params() {
    let health = health_check_params(&local_cache(), &overrides());
    assert_eq!(health.host, None);
    assert_eq!(health.port, None);
    assert_eq!(health.redis_kwargs, None);
    assert_eq!(health.namespace, None);
    assert_eq!(health.redis_version, None);
}

#[tokio::test]
async fn a_redis_cache_reports_its_connection_health_params() {
    let health = health_check_params(
        &redis_cache_from(unreachable_overrides()).await,
        &unreachable_overrides(),
    );
    assert_eq!(health.host, None);
    assert_eq!(health.port, None);
    assert_eq!(health.namespace, None);
    let kwargs = health.redis_kwargs.expect("the redis kwargs");
    assert_eq!(
        kwargs.get("host"),
        Some(&Json::Text("127.0.0.1".to_owned()))
    );
    assert_eq!(kwargs.get("port"), Some(&Json::Number(Number::from(1_i64))));
    assert_eq!(
        kwargs.get("socket_timeout"),
        Some(&Json::Number(Number::from_f64(5.0_f64).unwrap()))
    );
    let version = serde_json::to_string(&health.redis_version).unwrap();
    assert_eq!(version, "7");
}

#[tokio::test]
async fn a_redis_cache_masks_its_password_in_the_health_params() {
    let mut settings = unreachable_overrides();
    settings.password = Some(Some(SecretString::from("super-secret-password".to_owned())));
    let health = health_check_params(&redis_cache_from(settings.clone()).await, &settings);
    let kwargs = health.redis_kwargs.expect("the redis kwargs");
    let expected = SensitiveDataMasker::default().mask_value("super-secret-password");
    assert_eq!(kwargs.get("password"), Some(&Json::Text(expected)));
    assert_ne!(
        kwargs.get("password"),
        Some(&Json::Text("super-secret-password".to_owned()))
    );
}

#[tokio::test]
async fn a_redis_cache_health_params_carry_the_cache_namespace() {
    let mut cache = redis_cache_from(unreachable_overrides()).await;
    cache.namespace = Some("team-ns".to_owned());
    let health = health_check_params(&cache, &unreachable_overrides());
    assert_eq!(health.namespace.as_deref(), Some("team-ns"));
}
