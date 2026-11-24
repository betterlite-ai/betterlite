use std::error::Error as StdError;
use std::sync::Arc;

use betterlite_cache::configuration::settings::{
    DEFAULT_REDIS_MAJOR_VERSION, DEFAULT_REDIS_TTL_SECONDS,
};
use betterlite_config::loader::startup::process_environment;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::RedisCacheOptions;
use betterlite_redis::redis_cache::connect::connect;
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_live_client::LiveClientFactory;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::convert;
use super::error;
use super::settings::RedisSettings;

type AsyncAnswer<'py> = PyResult<Bound<'py, PyAny>>;

// RedisCache over the shared redis client; the fake server in the tests stands in for a live one.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeRedis {
    pub(crate) cache: Arc<RedisCache>,
}

#[pymethods]
impl NativeRedis {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let parsed: RedisSettings = convert::parse_settings(settings)?;
        let options = RedisCacheOptions {
            overrides: parsed.overrides,
            family: RedisCacheFamily::Standalone,
            namespace: parsed.namespace,
            default_ttl: parsed.default_ttl.unwrap_or(DEFAULT_REDIS_TTL_SECONDS),
            redis_flush_size: parsed.redis_flush_size,
            default_major_version: DEFAULT_REDIS_MAJOR_VERSION,
            factory: Arc::new(LiveClientFactory::new(None)),
            store: Arc::new(RedisClientStore::new()),
            hooks: Arc::new(QuietHooks),
            gate: Arc::new(QuietGate),
        };
        let environment = process_environment();
        let cache = error::backend_err(get_runtime().block_on(connect(options, &environment)))?;
        Ok(Self {
            cache: Arc::new(cache),
        })
    }

    // LiteLLM's sync set swallows a Redis failure after a debug line; so does the values slice.
    fn set(&self, key: &str, value: &str, ttl: Option<f64>) -> PyResult<()> {
        let json = convert::json_value(value)?;
        self.cache.values.set_cache(key, &json, ttl);
        Ok(())
    }

    fn async_set<'py>(
        &self,
        py: Python<'py>,
        key: &str,
        value: &str,
        ttl: Option<f64>,
    ) -> AsyncAnswer<'py> {
        let json = convert::json_value(value)?;
        let cache = Arc::clone(&self.cache);
        let owned = key.to_owned();
        future_into_py(py, async move {
            error::backend_err(
                cache
                    .values
                    .async_set_cache(Some(&owned), &json, ttl, false)
                    .await,
            )?;
            Ok(())
        })
    }

    fn get(&self, key: &str) -> PyResult<Option<String>> {
        let stored = error::backend_err(self.cache.values.get_cache(key))?;
        stored
            .map(convert::text_of_cached)
            .transpose()
            .map_err(PyErr::from)
    }

    fn async_get<'py>(&self, py: Python<'py>, key: &str) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        let owned = key.to_owned();
        future_into_py(py, async move {
            let stored = error::backend_err(cache.async_get_cache(&owned).await)?;
            stored
                .map(convert::text_of_cached)
                .transpose()
                .map_err(PyErr::from)
        })
    }

    // LiteLLM's sync increment is typed int and uses INCR; the counter slice keeps those whole numbers.
    fn increment(&self, key: &str, value: f64, ttl: Option<f64>) -> PyResult<String> {
        let whole = convert::whole_increment(value)?;
        let counted = error::backend_err(self.cache.counters.increment_cache(key, whole, ttl))?;
        convert::whole_text(counted).map_err(PyErr::from)
    }

    fn async_increment<'py>(
        &self,
        py: Python<'py>,
        key: &str,
        value: f64,
        ttl: Option<f64>,
    ) -> AsyncAnswer<'py> {
        let seconds = ttl.map(convert::whole_seconds).transpose()?;
        let cache = Arc::clone(&self.cache);
        let owned = key.to_owned();
        future_into_py(py, async move {
            let counted =
                error::backend_err(cache.async_increment(&owned, value, seconds, false).await)?;
            convert::number_text(counted).map_err(PyErr::from)
        })
    }

    fn delete(&self, keys: Vec<String>) -> PyResult<()> {
        for key in &keys {
            error::backend_err(self.cache.admin.delete_cache(key))?;
        }
        Ok(())
    }

    fn async_delete<'py>(&self, py: Python<'py>, keys: Vec<String>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            error::backend_err(cache.admin.delete_cache_keys(&keys).await)?;
            Ok(())
        })
    }

    fn ttl(&self, py: Python<'_>, key: &str) -> PyResult<Option<String>> {
        let cache = Arc::clone(&self.cache);
        let remaining = py.detach(|| get_runtime().block_on(cache.async_get_ttl(key)));
        let seconds = error::backend_err(remaining)?;
        seconds
            .map(convert::whole_text)
            .transpose()
            .map_err(PyErr::from)
    }

    fn async_ttl<'py>(&self, py: Python<'py>, key: &str) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        let owned = key.to_owned();
        future_into_py(py, async move {
            let seconds = error::backend_err(cache.async_get_ttl(&owned).await)?;
            seconds
                .map(convert::whole_text)
                .transpose()
                .map_err(PyErr::from)
        })
    }

    fn flush(&self) -> PyResult<()> {
        error::backend_err(self.cache.admin.flush_cache())?;
        Ok(())
    }

    fn async_flush<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            error::backend_err(cache.admin.flush_cache())?;
            Ok(())
        })
    }

    // ast-grep-ignore: no-padded-one-line-functions - the sync probe keeps its failure and the async one asks the same question
    fn ping(&self) -> PyResult<bool> {
        let alive = error::backend_err(self.cache.admin.sync_ping(None))?;
        Ok(alive)
    }

    fn async_ping<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            let alive = error::backend_err(cache.admin.ping(None).await)?;
            Ok(alive)
        })
    }

    fn close(&self, py: Python<'_>) -> PyResult<()> {
        let cache = Arc::clone(&self.cache);
        py.detach(|| get_runtime().block_on(cache.admin.disconnect()));
        Ok(())
    }

    fn async_close<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            cache.admin.disconnect().await;
            Ok(())
        })
    }
}

struct QuietHooks;

impl ServiceHooks for QuietHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn StdError + Send + Sync)) {}
}

struct QuietGate;

impl BreakerGate for QuietGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0_u64))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn StdError + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn StdError + Send + Sync)) {}
}
