use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::sync::Arc;

use betterlite_cache::backend::CacheBackend;
use betterlite_cache::configuration::cache::{Backend, Cache};
use betterlite_cache::configuration::callback_registration::CacheCallbacks;
use betterlite_cache::configuration::settings::CacheDependencies;
use betterlite_cache::disk::Clock;
use betterlite_cache::embedding::embedding_cache::add_embedding_entry_to_cache;
use betterlite_cache::embedding::embedding_cache::pipeline::async_add_cache_pipeline;
use betterlite_cache::envelope::backend::{EnvelopeBackend, StoredBackend};
use betterlite_cache::envelope::facade::ResponseCache;
use betterlite_cache::types::cache_type::LiteLlmCacheType;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_live_client::LiveClientFactory;

use super::convert::{self, CacheCall};
use super::error::CacheError;
use super::settings::CacheSettingsWire;
use super::sink::{PythonConsole, PythonLog};

mod tests;

pub(crate) struct CacheCore {
    cache: Cache,
    facade: ResponseCache<PythonLog, PythonConsole>,
}

impl CacheCore {
    // Only the local backend becomes a native store: Backend::Redis and Backend::Disk are not CacheBackend implementations.
    pub(crate) async fn open(wire: &CacheSettingsWire) -> Result<Self, CacheError> {
        if wire.cache_type != LiteLlmCacheType::Local {
            return Err(CacheError::UnsupportedType {
                cache_type: wire.cache_type,
            });
        }
        let clock: Clock = Arc::new(convert::wall_clock);
        let opened = open_cache(wire, Arc::clone(&clock)).await?;
        let (cache, store) = into_injected(opened)?;
        let backend: Arc<dyn EnvelopeBackend> = Arc::new(StoredBackend { backend: store });
        let facade = convert::facade(wire, backend, cache.ttl, clock)?;
        Ok(Self { cache, facade })
    }

    pub(crate) fn cache_key(&self, call: &CacheCall) -> Result<String, CacheError> {
        let mut request = call.envelope_request()?;
        convert::resolve_key(&mut request, &self.facade.key_settings, &self.facade.hooks)
    }

    pub(crate) fn lookup(&self, call: &CacheCall) -> Result<String, CacheError> {
        convert::value_text(&self.read(call)?)
    }

    pub(crate) async fn async_lookup(&self, call: &CacheCall) -> Result<String, CacheError> {
        convert::value_text(&self.async_read(call).await?)
    }

    // No backend reports a vector similarity yet, so a semantic lookup answers the cached response alone.
    pub(crate) fn lookup_semantic(&self, call: &CacheCall) -> Result<String, CacheError> {
        convert::semantic_text(&self.read(call)?, None)
    }

    pub(crate) async fn async_lookup_semantic(
        &self,
        call: &CacheCall,
    ) -> Result<String, CacheError> {
        convert::semantic_text(&self.async_read(call).await?, None)
    }

    pub(crate) fn write(&self, call: &CacheCall) -> Result<(), CacheError> {
        let mut request = call.envelope_request()?;
        let result = call.result_value()?;
        self.facade.add_cache(&mut request, &result);
        Ok(())
    }

    pub(crate) async fn async_write(&self, call: &CacheCall) -> Result<(), CacheError> {
        let mut request = call.envelope_request()?;
        let result = call.result_value()?;
        self.facade
            .async_add_cache(None, &mut request, &result)
            .await;
        Ok(())
    }

    pub(crate) fn write_embedding(&self, call: &CacheCall) -> Result<String, CacheError> {
        let mut request = call.envelope_request()?;
        let result = call.embedding_result()?;
        let entry = add_embedding_entry_to_cache(
            &self.facade,
            &mut request,
            &result,
            call.input_value()?,
            call.result_index(),
        )?;
        convert::embedding_text(&entry)
    }

    pub(crate) async fn async_write_pipeline(&self, call: &CacheCall) -> Result<(), CacheError> {
        let mut request = call.envelope_request()?;
        let result = call.embedding_result()?;
        async_add_cache_pipeline(&self.facade, None, None, &mut request, &result).await;
        Ok(())
    }

    pub(crate) async fn async_batch_write(&self, call: &CacheCall) -> Result<(), CacheError> {
        let mut request = call.envelope_request()?;
        let result = call.result_value()?;
        self.facade.batch_cache_write(&mut request, &result).await?;
        Ok(())
    }

    pub(crate) async fn close(&self) -> Result<(), CacheError> {
        self.cache.disconnect().await.map_err(CacheError::from)
    }

    fn read(&self, call: &CacheCall) -> Result<Option<CacheValue>, CacheError> {
        let mut request = call.envelope_request()?;
        Ok(self.facade.get_cache(None, &mut request))
    }

    async fn async_read(&self, call: &CacheCall) -> Result<Option<CacheValue>, CacheError> {
        let mut request = call.envelope_request()?;
        Ok(self.facade.async_get_cache(None, &mut request).await)
    }
}

async fn open_cache(wire: &CacheSettingsWire, clock: Clock) -> Result<Cache, CacheError> {
    let dependencies = dependencies(clock);
    let mut callbacks = CacheCallbacks::default();
    let environment = BTreeMap::new();
    Cache::open(wire.settings(), &mut callbacks, &environment, &dependencies)
        .await
        .map_err(|source| CacheError::Construction {
            source: Box::new(source),
        })
}

// The local backend needs no redis client, so the dependencies carry the inert redis parts the settings type requires.
fn dependencies(clock: Clock) -> CacheDependencies {
    CacheDependencies {
        clock,
        redis_factory: Arc::new(LiveClientFactory::new(None)),
        redis_store: Arc::new(RedisClientStore::new()),
        redis_hooks: Arc::new(SilentHooks),
        redis_gate: Arc::new(ClosedGate),
    }
}

// The facade and the lifecycle calls share one store, so a ping, a delete and a lookup all see the same entries.
fn into_injected(opened: Cache) -> Result<(Cache, Arc<dyn CacheBackend>), CacheError> {
    let Cache {
        backend,
        cache_type,
        namespace,
        redis_flush_size,
        ttl,
        mode,
        supported_call_types,
        semantic_cache_scope,
    } = opened;
    let Backend::Memory(memory) = backend else {
        return Err(CacheError::UnsupportedType { cache_type });
    };
    let store: Arc<dyn CacheBackend> = Arc::new(*memory);
    let cache = Cache {
        backend: Backend::Injected(Arc::clone(&store)),
        cache_type,
        namespace,
        redis_flush_size,
        ttl,
        mode,
        supported_call_types,
        semantic_cache_scope,
    };
    Ok((cache, store))
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
