#![cfg(test)]

use super::support::{
    FakeRedis, dependencies, environment, messages, redis_settings, settings, setup, switches,
};
use crate::configuration::cache::{Backend, Cache};
use crate::configuration::callback_registration::{CACHE_CALLBACK, CacheCallbacks};
use crate::types::cache_type::LiteLlmCacheType;

#[tokio::test]
async fn enable_cache_builds_a_cache_and_registers_callbacks() {
    let mut module = switches();
    module
        .enable_cache(setup(
            settings(LiteLlmCacheType::Local),
            &environment(),
            &dependencies(),
        ))
        .await
        .expect("the cache is enabled");
    let cache = module.cache.as_ref().expect("an enabled cache");
    assert!(matches!(cache.backend, Backend::Memory(_)));
    assert_eq!(
        module.callbacks.input_callback,
        vec![CACHE_CALLBACK.to_owned()]
    );
    assert_eq!(
        module.callbacks.success_callback,
        vec![CACHE_CALLBACK.to_owned()]
    );
    assert_eq!(
        module.callbacks.async_success_callback,
        vec![CACHE_CALLBACK.to_owned()]
    );
    assert_eq!(
        messages(&module.hooks),
        vec![
            "LiteLLM: Enabling Cache".to_owned(),
            "LiteLLM: Cache enabled, type=local, namespace=None, ttl=None, mode=DefaultOn, supported_call_types=14"
                .to_owned()
        ]
    );
}

#[tokio::test]
async fn enable_cache_registers_callbacks_for_an_existing_cache() {
    let mut module = switches();
    let existing = Cache::open(
        settings(LiteLlmCacheType::Local),
        &mut module.callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("an existing cache");
    module.cache = Some(existing);
    module.callbacks = CacheCallbacks::default();
    module
        .enable_cache(setup(
            settings(LiteLlmCacheType::Local),
            &environment(),
            &dependencies(),
        ))
        .await
        .expect("the cache stays enabled");
    assert_eq!(
        module.callbacks.input_callback,
        vec![CACHE_CALLBACK.to_owned()]
    );
}

#[tokio::test]
async fn enable_cache_keeps_an_existing_cache() {
    let mut module = switches();
    module
        .enable_cache(setup(
            settings(LiteLlmCacheType::Local),
            &environment(),
            &dependencies(),
        ))
        .await
        .expect("the cache is enabled");
    let directory = tempfile::tempdir().expect("a temp dir");
    let mut disk = settings(LiteLlmCacheType::Disk);
    disk.disk_cache_dir = Some(directory.path().to_owned());
    module
        .enable_cache(setup(disk, &environment(), &dependencies()))
        .await
        .expect("the cache stays enabled");
    let cache = module.cache.as_ref().expect("an enabled cache");
    assert_eq!(cache.cache_type, LiteLlmCacheType::Local);
    assert!(matches!(cache.backend, Backend::Memory(_)));
    assert_eq!(
        module.callbacks.input_callback,
        vec![CACHE_CALLBACK.to_owned()]
    );
}

#[tokio::test]
async fn update_cache_replaces_the_cache_and_registers_callbacks() {
    let mut module = switches();
    module
        .enable_cache(setup(
            settings(LiteLlmCacheType::Local),
            &environment(),
            &dependencies(),
        ))
        .await
        .expect("the cache is enabled");
    let directory = tempfile::tempdir().expect("a temp dir");
    let mut disk = settings(LiteLlmCacheType::Disk);
    disk.disk_cache_dir = Some(directory.path().to_owned());
    let previous = module
        .update_cache(setup(disk, &environment(), &dependencies()))
        .await
        .expect("the cache is updated")
        .expect("the replaced cache");
    assert_eq!(previous.cache_type, LiteLlmCacheType::Local);
    let cache = module.cache.as_ref().expect("an updated cache");
    assert_eq!(cache.cache_type, LiteLlmCacheType::Disk);
    assert!(matches!(cache.backend, Backend::Disk(_)));
    assert_eq!(
        module.callbacks.input_callback,
        vec![CACHE_CALLBACK.to_owned()]
    );
    assert_eq!(
        messages(&module.hooks),
        vec![
            "LiteLLM: Enabling Cache".to_owned(),
            "LiteLLM: Cache enabled, type=local, namespace=None, ttl=None, mode=DefaultOn, supported_call_types=14"
                .to_owned(),
            "LiteLLM: Updating Cache".to_owned(),
            "LiteLLM: Cache Updated, type=disk, namespace=None, ttl=None, mode=DefaultOn, supported_call_types=14"
                .to_owned()
        ]
    );
}

#[tokio::test]
async fn update_cache_keeps_the_previous_cache_when_construction_fails() {
    let mut module = switches();
    module
        .enable_cache(setup(
            settings(LiteLlmCacheType::Local),
            &environment(),
            &dependencies(),
        ))
        .await
        .expect("the cache is enabled");
    let result = module
        .update_cache(setup(
            settings(LiteLlmCacheType::S3),
            &environment(),
            &dependencies(),
        ))
        .await;
    assert!(result.is_err());
    let cache = module.cache.as_ref().expect("the previous cache");
    assert_eq!(cache.cache_type, LiteLlmCacheType::Local);
}

#[tokio::test]
async fn disable_cache_clears_the_cache_and_callbacks() {
    let mut module = switches();
    module
        .enable_cache(setup(
            settings(LiteLlmCacheType::Local),
            &environment(),
            &dependencies(),
        ))
        .await
        .expect("the cache is enabled");
    module.disable_cache();
    assert!(module.cache.is_none());
    assert!(module.callbacks.input_callback.is_empty());
    assert!(module.callbacks.success_callback.is_empty());
    assert!(module.callbacks.async_success_callback.is_empty());
    assert_eq!(
        messages(&module.hooks),
        vec![
            "LiteLLM: Enabling Cache".to_owned(),
            "LiteLLM: Cache enabled, type=local, namespace=None, ttl=None, mode=DefaultOn, supported_call_types=14"
                .to_owned(),
            "LiteLLM: Disabling Cache".to_owned(),
            "LiteLLM: Cache disabled".to_owned()
        ]
    );
}

#[tokio::test]
async fn disable_cache_tolerates_missing_entries() {
    let mut module = switches();
    module.disable_cache();
    assert!(module.cache.is_none());
    assert!(module.callbacks.input_callback.is_empty());
    assert_eq!(
        messages(&module.hooks),
        vec![
            "LiteLLM: Disabling Cache".to_owned(),
            "LiteLLM: Cache disabled".to_owned()
        ]
    );
}

#[tokio::test]
async fn enable_cache_logs_the_redis_type() {
    let server = FakeRedis::start().await;
    let mut module = switches();
    module
        .enable_cache(setup(
            redis_settings(&server),
            &environment(),
            &dependencies(),
        ))
        .await
        .expect("the cache is enabled");
    assert_eq!(
        messages(&module.hooks),
        vec![
            "LiteLLM: Enabling Cache".to_owned(),
            "LiteLLM: Cache enabled, type=redis, namespace=None, ttl=None, mode=DefaultOn, supported_call_types=14"
                .to_owned()
        ]
    );
}

#[tokio::test]
async fn disable_cache_removes_one_registration_from_each_list() {
    let mut module = switches();
    module.callbacks = CacheCallbacks {
        input_callback: vec![CACHE_CALLBACK.to_owned(), CACHE_CALLBACK.to_owned()],
        success_callback: Vec::new(),
        async_success_callback: vec!["langfuse".to_owned()],
    };
    module.disable_cache();
    assert_eq!(
        module.callbacks.input_callback,
        vec![CACHE_CALLBACK.to_owned()]
    );
    assert!(module.callbacks.success_callback.is_empty());
    assert_eq!(
        module.callbacks.async_success_callback,
        vec!["langfuse".to_owned()]
    );
}
