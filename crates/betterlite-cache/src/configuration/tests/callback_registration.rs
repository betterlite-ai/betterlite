#![cfg(test)]

use super::support::{dependencies, environment, settings, test_backend};
use crate::configuration::cache::Cache;
use crate::configuration::callback_registration::{CACHE_CALLBACK, CacheCallbacks};
use crate::types::cache_type::LiteLlmCacheType;

#[tokio::test]
async fn creating_a_cache_registers_the_cache_callback_in_every_list() {
    let mut callbacks = CacheCallbacks::default();
    let _cache = Cache::open(
        settings(LiteLlmCacheType::Local),
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("a local cache");
    assert_eq!(callbacks.input_callback, vec![CACHE_CALLBACK.to_owned()]);
    assert_eq!(callbacks.success_callback, vec![CACHE_CALLBACK.to_owned()]);
    assert_eq!(
        callbacks.async_success_callback,
        vec![CACHE_CALLBACK.to_owned()]
    );
}

#[tokio::test]
async fn a_second_cache_reuses_the_existing_registration() {
    let mut callbacks = CacheCallbacks::default();
    let _first = Cache::open(
        settings(LiteLlmCacheType::Local),
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("a first cache");
    let _second = Cache::open(
        settings(LiteLlmCacheType::Local),
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("a second cache");
    assert_eq!(callbacks.input_callback, vec![CACHE_CALLBACK.to_owned()]);
    assert_eq!(callbacks.success_callback, vec![CACHE_CALLBACK.to_owned()]);
    assert_eq!(
        callbacks.async_success_callback,
        vec![CACHE_CALLBACK.to_owned()]
    );
}

#[tokio::test]
async fn existing_callbacks_are_kept_and_the_cache_is_appended() {
    let mut callbacks = CacheCallbacks {
        input_callback: vec!["langfuse".to_owned()],
        success_callback: vec!["otel".to_owned()],
        async_success_callback: vec![CACHE_CALLBACK.to_owned()],
    };
    let _cache = Cache::open(
        settings(LiteLlmCacheType::Local),
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("a local cache");
    assert_eq!(
        callbacks.input_callback,
        vec!["langfuse".to_owned(), CACHE_CALLBACK.to_owned()]
    );
    assert_eq!(
        callbacks.success_callback,
        vec!["otel".to_owned(), CACHE_CALLBACK.to_owned()]
    );
    assert_eq!(
        callbacks.async_success_callback,
        vec![CACHE_CALLBACK.to_owned()]
    );
}

#[tokio::test]
async fn a_failed_construction_registers_nothing() {
    let mut callbacks = CacheCallbacks::default();
    let _error = Cache::open(
        settings(LiteLlmCacheType::S3),
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .err()
    .expect("an unsupported cache type");
    assert_eq!(callbacks, CacheCallbacks::default());
}

#[tokio::test]
async fn an_injected_backend_registers_the_cache_callback() {
    let (injected, _disconnects) = test_backend();
    let mut injected_settings = settings(LiteLlmCacheType::Local);
    injected_settings.backend = Some(injected);
    let mut callbacks = CacheCallbacks::default();
    let _cache = Cache::open(
        injected_settings,
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("an injected cache");
    assert_eq!(callbacks.input_callback, vec![CACHE_CALLBACK.to_owned()]);
    assert_eq!(callbacks.success_callback, vec![CACHE_CALLBACK.to_owned()]);
    assert_eq!(
        callbacks.async_success_callback,
        vec![CACHE_CALLBACK.to_owned()]
    );
}

#[test]
fn unregistering_removes_one_entry_and_tolerates_absence() {
    let mut callbacks = CacheCallbacks {
        input_callback: vec![CACHE_CALLBACK.to_owned(), CACHE_CALLBACK.to_owned()],
        success_callback: Vec::new(),
        async_success_callback: vec!["langfuse".to_owned()],
    };
    callbacks.unregister_cache();
    assert_eq!(callbacks.input_callback, vec![CACHE_CALLBACK.to_owned()]);
    assert!(callbacks.success_callback.is_empty());
    assert_eq!(
        callbacks.async_success_callback,
        vec!["langfuse".to_owned()]
    );
}
