use std::error::Error as StdError;
use std::sync::Arc;

use betterlite_cache::configuration::settings::{
    DEFAULT_REDIS_MAJOR_VERSION, DEFAULT_REDIS_TTL_SECONDS,
};
use betterlite_config::loader::startup::process_environment;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
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

use super::error::{self, BackendHandleError};
use super::settings::RedisSettings;
use super::{blocking, convert};

type AsyncAnswer<'py> = PyResult<Bound<'py, PyAny>>;

// RedisCache over the shared redis client; the fake server in the tests stands in for a live one.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeRedis {
    pub(crate) cache: Arc<RedisCache>,
    settings: RedisSettings,
}

pub(super) fn open(py: Python<'_>, parsed: RedisSettings) -> PyResult<NativeRedis> {
    let options = RedisCacheOptions {
        overrides: parsed.overrides.clone(),
        family: RedisCacheFamily::Standalone,
        namespace: parsed.namespace.clone(),
        default_ttl: parsed.default_ttl.unwrap_or(DEFAULT_REDIS_TTL_SECONDS),
        redis_flush_size: parsed.redis_flush_size,
        default_major_version: DEFAULT_REDIS_MAJOR_VERSION,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(QuietHooks),
        gate: Arc::new(QuietGate),
    };
    let environment = process_environment();
    let opened = py.detach(|| get_runtime().block_on(connect(options, &environment)));
    Ok(NativeRedis {
        cache: Arc::new(error::backend_err(opened)?),
        settings: parsed,
    })
}

#[pymethods]
impl NativeRedis {
    #[new]
    // ast-grep-ignore: no-one-line-functions - Python calls this constructor, so there is no Rust caller to inline into
    fn new(py: Python<'_>, settings: &str) -> PyResult<Self> {
        open(py, convert::parse_settings(settings)?)
    }

    // LiteLLM's sync set swallows a Redis failure after a debug line; so does the values slice.
    fn set(&self, py: Python<'_>, key: &str, value: &str, ttl: Option<f64>) -> PyResult<()> {
        let json = convert::json_value(value)?;
        py.detach(|| self.cache.values.set_cache(key, &json, ttl));
        Ok(())
    }

    // The pyo3 signature is the whole Python contract, so it stays on one line.
    #[rustfmt::skip]
    #[pyo3(signature = (key, value, ttl, nx = false))]
    #[expect(clippy::too_many_arguments, reason = "PyO3 exposes Redis SET parameters individually")]
    fn async_set<'py>(&self, py: Python<'py>, key: &str, value: &str, ttl: Option<f64>, nx: bool) -> AsyncAnswer<'py> {
        let json = convert::json_value(value)?;
        let (cache, owned) = (Arc::clone(&self.cache), key.to_owned());
        future_into_py(py, store(cache, owned, json, ttl, nx))
    }

    fn get(&self, py: Python<'_>, key: &str) -> PyResult<Option<String>> {
        let stored = error::backend_err(py.detach(|| self.cache.values.get_cache(key)))?;
        stored
            .map(convert::text_of_cached)
            .transpose()
            .map_err(PyErr::from)
    }

    fn async_get<'py>(&self, py: Python<'py>, key: &str) -> AsyncAnswer<'py> {
        let (cache, owned) = (Arc::clone(&self.cache), key.to_owned());
        future_into_py(py, async move {
            let stored = error::backend_err(cache.async_get_cache(&owned).await)?;
            answer_text(stored)
        })
    }

    // LiteLLM's sync increment is typed int and uses INCR; the counter slice keeps those whole numbers.
    fn increment(
        &self,
        py: Python<'_>,
        key: &str,
        value: i64,
        ttl: Option<f64>,
    ) -> PyResult<String> {
        let counted =
            error::backend_err(py.detach(|| self.cache.counters.increment_cache(key, value, ttl)))?;
        Ok(counted.to_string())
    }

    #[rustfmt::skip]
    fn async_increment<'py>(&self, py: Python<'py>, key: &str, value: f64, ttl: Option<f64>) -> AsyncAnswer<'py> {
        let seconds = ttl.map(convert::whole_seconds).transpose()?;
        let (cache, owned) = (Arc::clone(&self.cache), key.to_owned());
        future_into_py(py, count(cache, owned, value, seconds))
    }

    fn delete(&self, py: Python<'_>, keys: Vec<String>) -> PyResult<()> {
        for key in keys {
            error::backend_err(py.detach(|| self.cache.admin.delete_cache(&key)))?;
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
        Ok(seconds.map(|value| value.to_string()))
    }

    fn async_ttl<'py>(&self, py: Python<'py>, key: &str) -> AsyncAnswer<'py> {
        let (cache, owned) = (Arc::clone(&self.cache), key.to_owned());
        future_into_py(py, async move {
            let seconds = error::backend_err(cache.async_get_ttl(&owned).await)?;
            Ok(seconds.map(|value| value.to_string()))
        })
    }

    fn flush(&self, py: Python<'_>) -> PyResult<()> {
        error::backend_err(py.detach(|| self.cache.admin.flush_cache()))?;
        Ok(())
    }

    fn async_flush<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        blocking::into_py(py, {
            let cache = Arc::clone(&self.cache);
            move || cache.admin.flush_cache()
        })
    }

    // ast-grep-ignore: no-padded-one-line-functions - the sync probe keeps its failure and the async one asks the same question
    fn ping(&self, py: Python<'_>) -> PyResult<bool> {
        let alive = error::backend_err(py.detach(|| self.cache.admin.sync_ping(None)))?;
        Ok(alive)
    }

    // LiteLLM's Cache sets a RedisCache's namespace and later keys carry the new prefix.
    fn set_namespace(&mut self, py: Python<'_>, namespace: Option<&str>) -> PyResult<()> {
        if self.settings.namespace.as_deref() == namespace {
            return Ok(());
        }
        let mut settings = self.settings.clone();
        settings.namespace = namespace.map(str::to_owned);
        *self = open(py, settings)?;
        Ok(())
    }

    fn async_ping<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            let alive = error::backend_err(cache.admin.ping(None).await)?;
            Ok(alive)
        })
    }
    // ast-grep-ignore: no-padded-one-line-functions - the disconnect future is the whole call
    fn close(&self, py: Python<'_>) {
        let cache = Arc::clone(&self.cache);
        py.detach(|| get_runtime().block_on(cache.admin.disconnect()));
    }

    fn async_close<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            cache.admin.disconnect().await;
            Ok(())
        })
    }
}

async fn store(
    cache: Arc<RedisCache>,
    key: String,
    json: Json,
    ttl: Option<f64>,
    nx: bool,
) -> PyResult<Option<bool>> {
    let written = cache
        .values
        .async_set_cache(Some(&key), &json, ttl, nx)
        .await;
    let stored = error::backend_err(written)?;
    Ok(stored)
}

fn answer_text(stored: Option<CachedValue>) -> PyResult<Option<String>> {
    stored.map_or_else(
        || Ok(None),
        |value| {
            convert::text_of_cached(value)
                .map(Some)
                .map_err(PyErr::from)
        },
    )
}

async fn count(cache: Arc<RedisCache>, key: String, by: f64, ttl: Option<i64>) -> PyResult<String> {
    let counted = error::backend_err(cache.async_increment(&key, by, ttl, false).await)?;
    let text = convert::number_text(counted).map_err(PyErr::from)?;
    Ok(text)
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

mod tests;
