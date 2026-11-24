use std::sync::Arc;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{
    CacheOptions, DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE, Ttl,
};
use betterlite_redis::redis_cache::RedisCache;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::convert;
use super::error::{self, BackendHandleError};
use super::memory::NativeMemory;
use super::redis::NativeRedis;
use super::settings::DualSettings;

type AsyncAnswer<'py> = PyResult<Bound<'py, PyAny>>;

// DualCache over the memory tier and, when one was handed in, the Redis tier.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeDual {
    cache: Arc<DualCache<LocalMemoryOps>>,
    redis: Option<Arc<RedisCache>>,
}

#[pymethods]
impl NativeDual {
    #[new]
    fn new(
        memory: PyRef<'_, NativeMemory>,
        redis: Option<PyRef<'_, NativeRedis>>,
        settings: &str,
    ) -> PyResult<Self> {
        let parsed: DualSettings = convert::parse_settings(settings)?;
        let max_size = parsed
            .max_redis_batch_cache_size
            .unwrap_or(MAX_REDIS_BATCH_CACHE_SIZE);
        let dual_settings = DualCacheSettings {
            default_in_memory_ttl: parsed.default_in_memory_ttl,
            default_redis_ttl: parsed.default_redis_ttl,
            batch_cache_expiry: parsed.batch_cache_expiry,
            max_redis_batch_cache_size: max_size,
            redis_delete_chunk_size: max_size,
        };
        let redis_tier = redis.map(|handle| Arc::clone(&handle.cache));
        let cache = DualCache::new(Arc::clone(&memory.ops), redis_tier.clone(), dual_settings);
        Ok(Self {
            cache: Arc::new(cache),
            redis: redis_tier,
        })
    }

    fn set(&self, key: &str, value: &str, ttl: Option<f64>) -> PyResult<()> {
        let json = convert::json_value(value)?;
        self.cache.set_cache(key, &json, options(ttl));
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
            cache.async_set_cache(&owned, &json, options(ttl)).await;
            Ok(())
        })
    }

    fn get(&self, key: &str) -> PyResult<Option<String>> {
        self.cache
            .get_cache(key, false)
            .as_ref()
            .map(convert::text_of_json)
            .transpose()
            .map_err(PyErr::from)
    }

    fn async_get<'py>(&self, py: Python<'py>, key: &str) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        let owned = key.to_owned();
        future_into_py(py, async move {
            cache
                .async_get_cache(&owned, false)
                .await
                .as_ref()
                .map(convert::text_of_json)
                .transpose()
                .map_err(PyErr::from)
        })
    }

    // ast-grep-ignore: no-padded-one-line-functions - one count to text for both the sync and async callers
    fn increment(&self, key: &str, value: f64, ttl: Option<f64>) -> PyResult<String> {
        let counted = error::backend_err(self.cache.increment_cache(key, value, options(ttl)))?;
        convert::number_text(counted).map_err(PyErr::from)
    }

    fn async_increment<'py>(
        &self,
        py: Python<'py>,
        key: &str,
        value: f64,
        ttl: Option<f64>,
    ) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        let owned = key.to_owned();
        future_into_py(py, async move {
            let counted = cache
                .async_increment_cache(&owned, value, options(ttl))
                .await
                .ok_or_else(|| PyErr::from(BackendHandleError::IncrementUnavailable))?;
            convert::number_text(counted).map_err(PyErr::from)
        })
    }

    fn delete(&self, keys: Vec<String>) -> PyResult<()> {
        for key in &keys {
            error::backend_err(self.cache.delete_cache(key))?;
        }
        Ok(())
    }

    fn async_delete<'py>(&self, py: Python<'py>, keys: Vec<String>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            error::backend_err(cache.async_delete_cache_keys(&keys).await)?;
            Ok(())
        })
    }

    fn ttl(&self, py: Python<'_>, key: &str) -> PyResult<Option<String>> {
        let cache = Arc::clone(&self.cache);
        let remaining = py.detach(|| get_runtime().block_on(cache.async_get_ttl(key)));
        let ttl = error::backend_err(remaining)?;
        text_of_ttl(ttl)
    }

    fn async_ttl<'py>(&self, py: Python<'py>, key: &str) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        let owned = key.to_owned();
        future_into_py(py, async move {
            let ttl = error::backend_err(cache.async_get_ttl(&owned).await)?;
            text_of_ttl(ttl)
        })
    }

    fn flush(&self) -> PyResult<()> {
        error::backend_err(self.cache.flush_cache())?;
        Ok(())
    }

    fn async_flush<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            error::backend_err(cache.flush_cache())?;
            Ok(())
        })
    }

    fn ping(&self, py: Python<'_>) -> PyResult<bool> {
        let Some(redis) = &self.redis else {
            return Ok(true);
        };
        let probe = Arc::clone(redis);
        let alive = py.detach(|| get_runtime().block_on(probe.admin.ping(None)));
        error::backend_err(alive)
    }

    fn async_ping<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let redis = self.redis.as_ref().map(Arc::clone);
        future_into_py(py, async move {
            let Some(tier) = redis else {
                return Ok(true);
            };
            let alive = error::backend_err(tier.admin.ping(None).await)?;
            Ok(alive)
        })
    }

    fn close(&self, py: Python<'_>) -> PyResult<()> {
        let Some(redis) = &self.redis else {
            return Ok(());
        };
        let tier = Arc::clone(redis);
        py.detach(|| get_runtime().block_on(tier.admin.disconnect()));
        Ok(())
    }

    fn async_close<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let redis = self.redis.as_ref().map(Arc::clone);
        future_into_py(py, async move {
            if let Some(tier) = redis {
                tier.admin.disconnect().await;
            }
            Ok(())
        })
    }
}

const fn options(ttl: Option<f64>) -> CacheOptions {
    CacheOptions {
        ttl,
        local_only: false,
        refresh_ttl: false,
    }
}

fn text_of_ttl(ttl: Option<Ttl>) -> PyResult<Option<String>> {
    ttl.map(|value| {
        serde_json::to_string(&value).map_err(|source| BackendHandleError::Encode { source })
    })
    .transpose()
    .map_err(PyErr::from)
}
