#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use rstest::rstest;
use thiserror::Error;

use super::{Cache, FactoryError};
use crate::backend::{CacheBackend, CacheResult, Ttl};
use crate::configuration::cache::{Backend, Cache as CacheFacade};
use crate::configuration::callback_registration::CacheCallbacks;
use crate::native_adapter::{
    MemoryHandleOptions, NativeCacheHandle, NativeHandleFactory, RedisHandleOptions,
};
use crate::types::cache_type::{CacheMode, LiteLlmCacheType};
use crate::types::call_type::SUPPORTED_CALL_TYPES;
use crate::types::semantic::SemanticCacheScope;

#[derive(Clone, Debug, PartialEq)]
enum Call {
    Get(String),
    Set(String, Ttl),
}

#[derive(Default)]
struct HandleState {
    calls: Vec<Call>,
    values: BTreeMap<String, CacheValue>,
}

#[derive(Default)]
struct RecordingHandle {
    state: Mutex<HandleState>,
}

impl RecordingHandle {
    fn state(&self) -> MutexGuard<'_, HandleState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl NativeCacheHandle for RecordingHandle {
    fn get(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        self.state().calls.push(Call::Get(key.to_owned()));
        Ok(self.state().values.get(key).cloned())
    }

    fn set(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.state().calls.push(Call::Set(key.to_owned(), ttl));
        self.state().values.insert(key.to_owned(), value);
        Ok(())
    }

    async fn async_get(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        self.get(key)
    }

    async fn async_set(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.set(key, value, ttl)
    }

    async fn async_set_many(
        &self,
        entries: Vec<(String, CacheValue)>,
        _ttl: Ttl,
    ) -> CacheResult<()> {
        for (key, value) in entries {
            self.state().values.insert(key, value);
        }
        Ok(())
    }

    fn flush(&self) -> CacheResult<()> {
        self.state().values.clear();
        Ok(())
    }

    async fn async_flush(&self) -> CacheResult<()> {
        self.state().values.clear();
        Ok(())
    }

    async fn ping(&self) -> CacheResult<bool> {
        Ok(true)
    }

    async fn disconnect(&self) -> CacheResult<()> {
        Ok(())
    }

    async fn delete(&self, keys: &[String]) -> CacheResult<()> {
        for key in keys {
            self.state().values.remove(key);
        }
        Ok(())
    }
}

#[derive(Default)]
struct HostState {
    memory: Vec<MemoryHandleOptions>,
    redis: Vec<(String, String, RedisHandleOptions)>,
    handles: Vec<Arc<RecordingHandle>>,
}

#[derive(Default)]
struct RecordingHost {
    state: Mutex<HostState>,
    refusing: bool,
}

impl RecordingHost {
    fn state(&self) -> MutexGuard<'_, HostState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record_new(&self) -> Arc<RecordingHandle> {
        let handle = Arc::new(RecordingHandle::default());
        let retained: Arc<RecordingHandle> = Arc::clone(&handle);
        self.state().handles.push(retained);
        handle
    }
}

#[derive(Debug, Error)]
#[error("the fake native host refused the handle")]
struct HostError;

impl NativeHandleFactory for RecordingHost {
    type Error = HostError;

    fn memory_handle(
        &self,
        options: MemoryHandleOptions,
    ) -> Result<Arc<dyn NativeCacheHandle>, HostError> {
        if self.refusing {
            return Err(HostError);
        }
        self.state().memory.push(options);
        let handle: Arc<dyn NativeCacheHandle> = self.record_new();
        Ok(handle)
    }

    fn redis_handle(
        &self,
        url: &str,
        namespace: &str,
        options: RedisHandleOptions,
    ) -> Result<Arc<dyn NativeCacheHandle>, HostError> {
        let recorded = (url.to_owned(), namespace.to_owned(), options);
        if self.refusing {
            return Err(HostError);
        }
        self.state().redis.push(recorded);
        Ok(self.record_new())
    }
}

fn injected(facade: &CacheFacade) -> Arc<dyn CacheBackend> {
    match &facade.backend {
        Backend::Injected(backend) => Arc::clone(backend),
        Backend::Memory(_) | Backend::Redis(_) | Backend::Disk(_) => {
            panic!("the v2 factory injects its native backend")
        }
    }
}

#[test]
fn memory_creates_a_local_facade_with_the_litellm_defaults() {
    let host = RecordingHost::default();
    let mut callbacks = CacheCallbacks::default();
    let facade = Cache::memory(&host, &mut callbacks, MemoryHandleOptions::default()).unwrap();
    assert_eq!(
        host.state().memory,
        [MemoryHandleOptions {
            ttl: 600.0,
            capacity: 200,
            max_entry_bytes: 0x0040_0000,
        }]
    );
    assert_eq!(facade.cache_type, LiteLlmCacheType::Local);
    assert_eq!(facade.ttl, Some(600.0_f64));
    assert_eq!(facade.namespace, None);
    assert_eq!(facade.redis_flush_size, None);
    assert_eq!(facade.mode, CacheMode::DefaultOn);
    assert_eq!(facade.supported_call_types, SUPPORTED_CALL_TYPES);
    assert_eq!(facade.semantic_cache_scope, SemanticCacheScope::Key);
    assert_eq!(
        callbacks,
        CacheCallbacks {
            input_callback: vec!["cache".to_owned()],
            success_callback: vec!["cache".to_owned()],
            async_success_callback: vec!["cache".to_owned()],
        }
    );
    assert_eq!(host.state().handles.len(), 1);
}

#[test]
fn memory_forwards_custom_options_to_the_handle() {
    let host = RecordingHost::default();
    let options = MemoryHandleOptions {
        ttl: 30.0,
        capacity: 7,
        max_entry_bytes: 1024,
    };
    let facade = Cache::memory(&host, &mut CacheCallbacks::default(), options).unwrap();
    assert_eq!(host.state().memory, [options]);
    assert_eq!(facade.ttl, Some(30.0_f64));
}

#[test]
fn memory_wires_the_facade_to_the_native_backend() {
    let host = RecordingHost::default();
    let facade = Cache::memory(
        &host,
        &mut CacheCallbacks::default(),
        MemoryHandleOptions::default(),
    )
    .unwrap();
    let backend = injected(&facade);
    backend
        .set_cache("key", CacheValue::Integer(7), None)
        .unwrap();
    assert_eq!(
        backend.get_cache("key").unwrap(),
        Some(CacheValue::Integer(7))
    );
    assert_eq!(
        host.state().handles[0].state().calls,
        [
            Call::Set("key".to_owned(), None),
            Call::Get("key".to_owned())
        ]
    );
}

#[test]
fn memory_registers_the_cache_callback_once() {
    let host = RecordingHost::default();
    let mut callbacks = CacheCallbacks::default();
    Cache::memory(&host, &mut callbacks, MemoryHandleOptions::default()).unwrap();
    Cache::memory(&host, &mut callbacks, MemoryHandleOptions::default()).unwrap();
    assert_eq!(callbacks.input_callback, ["cache".to_owned()]);
    assert_eq!(callbacks.success_callback, ["cache".to_owned()]);
    assert_eq!(callbacks.async_success_callback, ["cache".to_owned()]);
}

#[test]
fn redis_creates_a_redis_facade_with_the_required_namespace() {
    let host = RecordingHost::default();
    let mut callbacks = CacheCallbacks::default();
    let facade = Cache::redis(
        &host,
        &mut callbacks,
        "redis://cache:6379",
        "shared",
        RedisHandleOptions::default(),
    )
    .unwrap();
    assert_eq!(
        host.state().redis,
        [(
            "redis://cache:6379".to_owned(),
            "shared".to_owned(),
            RedisHandleOptions {
                ttl: 600.0,
                max_entry_bytes: 0x0040_0000,
            },
        )]
    );
    assert_eq!(facade.cache_type, LiteLlmCacheType::Redis);
    assert_eq!(facade.namespace, Some("shared".to_owned()));
    assert_eq!(facade.ttl, Some(600.0_f64));
    assert_eq!(callbacks.input_callback, ["cache".to_owned()]);
    assert_eq!(host.state().handles.len(), 1);
}

#[test]
fn redis_forwards_custom_options_to_the_handle() {
    let host = RecordingHost::default();
    let options = RedisHandleOptions {
        ttl: 45.0,
        max_entry_bytes: 2_048,
    };
    let facade = Cache::redis(
        &host,
        &mut CacheCallbacks::default(),
        "redis://cache:6379",
        "tenant",
        options,
    )
    .unwrap();
    assert_eq!(
        host.state().redis,
        [(
            "redis://cache:6379".to_owned(),
            "tenant".to_owned(),
            options
        )]
    );
    assert_eq!(facade.ttl, Some(45.0_f64));
    assert_eq!(facade.namespace, Some("tenant".to_owned()));
}

#[test]
fn redis_rejects_an_empty_namespace_before_the_handle_is_created() {
    let host = RecordingHost::default();
    let mut callbacks = CacheCallbacks::default();
    let Err(error) = Cache::redis(
        &host,
        &mut callbacks,
        "redis://cache:6379",
        "",
        RedisHandleOptions::default(),
    ) else {
        panic!("the factory must reject an empty namespace")
    };
    assert!(matches!(error, FactoryError::Namespace));
    assert_eq!(callbacks, CacheCallbacks::default());
    assert!(host.state().redis.is_empty());
    assert!(host.state().handles.is_empty());
}

#[derive(Clone, Copy, Debug)]
enum FactoryKind {
    Memory,
    Redis,
}

#[rstest]
#[case::memory_zero(FactoryKind::Memory, 0.0_f64)]
#[case::redis_zero(FactoryKind::Redis, 0.0_f64)]
#[case::memory_negative(FactoryKind::Memory, -1.0_f64)]
#[case::redis_negative(FactoryKind::Redis, -1.0_f64)]
#[case::redis_nan(FactoryKind::Redis, f64::NAN)]
#[case::redis_positive_infinity(FactoryKind::Redis, f64::INFINITY)]
#[case::redis_negative_infinity(FactoryKind::Redis, f64::NEG_INFINITY)]
#[case::memory_sub_nanosecond(FactoryKind::Memory, 1e-10_f64)]
#[case::redis_sub_nanosecond(FactoryKind::Redis, 1e-10_f64)]
#[case::memory_overflow(FactoryKind::Memory, 1e20_f64)]
#[case::redis_overflow(FactoryKind::Redis, 1e20_f64)]
fn an_invalid_ttl_is_rejected_before_the_handle_is_created(
    #[case] factory: FactoryKind,
    #[case] ttl: f64,
) {
    let host = RecordingHost::default();
    let mut callbacks = CacheCallbacks::default();
    let result = match factory {
        FactoryKind::Memory => Cache::memory(
            &host,
            &mut callbacks,
            MemoryHandleOptions {
                ttl,
                capacity: 200,
                max_entry_bytes: 0x0040_0000,
            },
        ),
        FactoryKind::Redis => Cache::redis(
            &host,
            &mut callbacks,
            "redis://cache:6379",
            "shared",
            RedisHandleOptions {
                ttl,
                max_entry_bytes: 0x0040_0000,
            },
        ),
    };
    let Err(error) = result else {
        panic!("the factory must reject an invalid ttl")
    };
    let FactoryError::Ttl { value } = error else {
        panic!("the factory must report the invalid ttl")
    };
    assert_eq!(value.to_bits(), ttl.to_bits());
    assert_eq!(callbacks, CacheCallbacks::default());
    assert!(host.state().memory.is_empty());
    assert!(host.state().redis.is_empty());
    assert!(host.state().handles.is_empty());
}

#[rstest]
#[case::one_nanosecond(1e-9_f64)]
#[case::one_billion_seconds(1e9_f64)]
fn a_representable_positive_ttl_reaches_the_handle(#[case] ttl: f64) {
    let host = RecordingHost::default();
    let facade = Cache::memory(
        &host,
        &mut CacheCallbacks::default(),
        MemoryHandleOptions {
            ttl,
            capacity: 200,
            max_entry_bytes: 0x0040_0000,
        },
    )
    .unwrap();
    assert_eq!(facade.ttl, Some(ttl));
    assert_eq!(
        host.state().memory,
        [MemoryHandleOptions {
            ttl,
            capacity: 200,
            max_entry_bytes: 0x0040_0000,
        }]
    );
}

#[rstest]
#[case::memory_capacity_zero(FactoryKind::Memory, 0_usize, 0x0040_0000_usize)]
#[case::memory_max_entry_zero(FactoryKind::Memory, 200_usize, 0_usize)]
#[case::redis_max_entry_zero(FactoryKind::Redis, 200_usize, 0_usize)]
fn a_zero_limit_is_rejected_before_the_handle_is_created(
    #[case] factory: FactoryKind,
    #[case] capacity: usize,
    #[case] max_entry_bytes: usize,
) {
    let host = RecordingHost::default();
    let mut callbacks = CacheCallbacks::default();
    let result = match factory {
        FactoryKind::Memory => Cache::memory(
            &host,
            &mut callbacks,
            MemoryHandleOptions {
                ttl: 600.0,
                capacity,
                max_entry_bytes,
            },
        ),
        FactoryKind::Redis => Cache::redis(
            &host,
            &mut callbacks,
            "redis://cache:6379",
            "shared",
            RedisHandleOptions {
                ttl: 600.0,
                max_entry_bytes,
            },
        ),
    };
    let Err(error) = result else {
        panic!("the factory must reject a zero limit")
    };
    assert!(matches!(error, FactoryError::Limits));
    assert_eq!(callbacks, CacheCallbacks::default());
    assert!(host.state().memory.is_empty());
    assert!(host.state().redis.is_empty());
    assert!(host.state().handles.is_empty());
}

#[tokio::test]
async fn the_facade_lifecycle_does_not_reach_the_native_backend() {
    let host = RecordingHost::default();
    let facade = Cache::memory(
        &host,
        &mut CacheCallbacks::default(),
        MemoryHandleOptions::default(),
    )
    .unwrap();
    assert_eq!(facade.ping().await.unwrap(), None);
    facade.delete_cache_keys(&["key".to_owned()]).await.unwrap();
    assert!(host.state().handles[0].state().calls.is_empty());
}

#[rstest]
#[case::memory(FactoryKind::Memory)]
#[case::redis(FactoryKind::Redis)]
fn a_host_failure_is_returned_with_its_source(#[case] factory: FactoryKind) {
    let host = RecordingHost {
        state: Mutex::new(HostState::default()),
        refusing: true,
    };
    let mut callbacks = CacheCallbacks::default();
    let result = match factory {
        FactoryKind::Memory => {
            Cache::memory(&host, &mut callbacks, MemoryHandleOptions::default()).map(drop)
        }
        FactoryKind::Redis => Cache::redis(
            &host,
            &mut callbacks,
            "redis://cache:6379",
            "shared",
            RedisHandleOptions::default(),
        )
        .map(drop),
    };
    let Err(error) = result else {
        panic!("the factory must return the host failure")
    };
    assert!(matches!(error, FactoryError::Handle { source: HostError }));
    assert_eq!(callbacks, CacheCallbacks::default());
    assert!(host.state().memory.is_empty());
    assert!(host.state().redis.is_empty());
    assert!(host.state().handles.is_empty());
}

#[test]
fn no_handle_exists_before_a_factory_is_called() {
    let host = RecordingHost::default();
    assert!(host.state().handles.is_empty());
    Cache::memory(
        &host,
        &mut CacheCallbacks::default(),
        MemoryHandleOptions::default(),
    )
    .unwrap();
    assert_eq!(host.state().handles.len(), 1);
}

#[test]
fn each_factory_call_builds_its_own_handle() {
    let host = RecordingHost::default();
    let mut callbacks = CacheCallbacks::default();
    let first = Cache::memory(&host, &mut callbacks, MemoryHandleOptions::default()).unwrap();
    let second = Cache::memory(&host, &mut callbacks, MemoryHandleOptions::default()).unwrap();
    let handles = host.state().handles.clone();
    assert_eq!(handles.len(), 2);
    assert!(!Arc::ptr_eq(&handles[0], &handles[1]));
    injected(&first)
        .set_cache("key", CacheValue::Integer(1), None)
        .unwrap();
    assert_eq!(injected(&second).get_cache("key").unwrap(), None);
}
