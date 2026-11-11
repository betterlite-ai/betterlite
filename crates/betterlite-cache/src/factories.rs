use std::error::Error;
use std::sync::Arc;

use thiserror::Error;

use crate::configuration::cache::{Backend, Cache as CacheFacade};
use crate::configuration::callback_registration::CacheCallbacks;
use crate::native_adapter::{
    MemoryHandleOptions, NativeBackend, NativeCacheHandle, NativeHandleFactory, RedisHandleOptions,
};
use crate::types::cache_type::{CacheMode, LiteLlmCacheType};
use crate::types::call_type::SUPPORTED_CALL_TYPES;
use crate::types::semantic::SemanticCacheScope;

#[derive(Debug, Error)]
pub enum FactoryError<HandleError>
where
    HandleError: Error + 'static,
{
    #[error("the native cache ttl {value} is not a finite positive duration")]
    Ttl { value: f64 },
    #[error("the native redis cache needs a non-empty namespace")]
    Namespace,
    #[error("the native cache handle could not be created")]
    Handle {
        #[source]
        source: HandleError,
    },
}

pub struct Cache;

impl Cache {
    pub fn memory<Factory>(
        factory: &Factory,
        callbacks: &mut CacheCallbacks,
        options: MemoryHandleOptions,
    ) -> Result<CacheFacade, FactoryError<Factory::Error>>
    where
        Factory: NativeHandleFactory,
    {
        checked_ttl(options.ttl)?;
        let handle = factory
            .memory_handle(options)
            .map_err(|source| FactoryError::Handle { source })?;
        Ok(finish(
            handle,
            callbacks,
            LiteLlmCacheType::Local,
            None,
            options.ttl,
        ))
    }

    pub fn redis<Factory>(
        factory: &Factory,
        callbacks: &mut CacheCallbacks,
        url: &str,
        namespace: &str,
        options: RedisHandleOptions,
    ) -> Result<CacheFacade, FactoryError<Factory::Error>>
    where
        Factory: NativeHandleFactory,
    {
        checked_ttl(options.ttl)?;
        if namespace.is_empty() {
            return Err(FactoryError::Namespace);
        }
        let handle = factory
            .redis_handle(url, namespace, options)
            .map_err(|source| FactoryError::Handle { source })?;
        Ok(finish(
            handle,
            callbacks,
            LiteLlmCacheType::Redis,
            Some(namespace.to_owned()),
            options.ttl,
        ))
    }
}

// the native handle refuses a zero or non-finite duration, so the factory refuses it before the handle exists
fn checked_ttl<HandleError>(ttl: f64) -> Result<(), FactoryError<HandleError>>
where
    HandleError: Error + 'static,
{
    if !ttl.is_finite() || ttl <= 0.0_f64 {
        return Err(FactoryError::Ttl { value: ttl });
    }
    Ok(())
}

// the state CacheFacade.__init__ stores for an injected _backend, with the defaults the v2 factories leave alone
fn finish(
    handle: Arc<dyn NativeCacheHandle>,
    callbacks: &mut CacheCallbacks,
    cache_type: LiteLlmCacheType,
    namespace: Option<String>,
    ttl: f64,
) -> CacheFacade {
    callbacks.register_cache();
    CacheFacade {
        backend: Backend::Injected(Arc::new(NativeBackend { handle })),
        cache_type,
        namespace,
        redis_flush_size: None,
        ttl: Some(ttl),
        mode: CacheMode::DefaultOn,
        supported_call_types: SUPPORTED_CALL_TYPES.to_vec(),
        semantic_cache_scope: SemanticCacheScope::Key,
    }
}

mod tests;
