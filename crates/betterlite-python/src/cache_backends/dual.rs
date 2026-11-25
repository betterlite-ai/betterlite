use std::sync::{Arc, Mutex};

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::batch_reads::BatchReadOptions;
use betterlite_cache::batch_reads::BatchReads;
use betterlite_cache::dual_cache::{
    CacheOptions, DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE, Ttl,
};
use betterlite_models::decisions::Json;
use betterlite_redis::batch::result::BatchResult;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::scope::active_request_redis_batch;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::convert;
use super::error::{self, BackendHandleError};
use super::memory::NativeMemory;
use super::redis::NativeRedis;
use super::settings::DualSettings;

type AsyncAnswer<'py> = PyResult<Bound<'py, PyAny>>;
type MemoryDual = DualCache<LocalMemoryOps>;

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeDeferred {
    result: Mutex<Option<BatchResult<()>>>,
}

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeDeclaredBatchRead {
    cache: Arc<MemoryDual>,
    declared: Mutex<Option<betterlite_cache::batch_reads::DeclaredBatchRead>>,
}

#[pymethods]
impl NativeDeferred {
    fn async_wait<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let result = match self.result.lock() {
            Ok(mut held) => held.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        let Some(result) = result else {
            return Err(PyRuntimeError::new_err(
                "deferred cache result was already consumed",
            ));
        };
        future_into_py(py, async move {
            result
                .await
                .map_err(|error| PyRuntimeError::new_err(error.to_string()))
        })
    }

    fn cancel(&self) {
        match self.result.lock() {
            Ok(mut held) => {
                if let Some(result) = held.take() {
                    result.cancel();
                }
            }
            Err(poisoned) => {
                if let Some(result) = poisoned.into_inner().take() {
                    result.cancel();
                }
            }
        }
    }
}

#[pymethods]
impl NativeDeclaredBatchRead {
    fn async_resolve<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let declared = match self.declared.lock() {
            Ok(mut held) => held.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        let Some(declared) = declared else {
            return Err(PyRuntimeError::new_err(
                "declared batch read was already consumed",
            ));
        };
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            BatchReads::async_resolve_batch_get(&*cache, declared)
                .await
                .map(|values| values.iter().map(answer_text).collect())
                .map_err(|error| PyRuntimeError::new_err(error.to_string()))
        })
    }
}

// DualCache over the memory tier and, when one was handed in, the Redis tier.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeDual {
    cache: Arc<MemoryDual>,
    redis: Option<Arc<RedisCache>>,
    // the memory tier's configured default, only when it differs from the tier's own 600 seconds
    memory_ttl: Option<f64>,
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
        let memory_ttl = memory.default_ttl;
        let dual_settings = DualCacheSettings {
            default_in_memory_ttl: parsed.default_in_memory_ttl.or(memory_ttl),
            default_redis_ttl: parsed.default_redis_ttl,
            batch_cache_expiry: parsed.batch_cache_expiry,
            max_redis_batch_cache_size: max_size,
            redis_delete_chunk_size: max_size,
        };
        let redis_tier = redis.map(|handle| Arc::clone(&handle.cache));
        let cache = DualCache::new(Arc::clone(&memory.ops), redis_tier.clone(), dual_settings);
        drop(memory);
        Ok(Self {
            cache: Arc::new(cache),
            redis: redis_tier,
            memory_ttl,
        })
    }

    fn set(&self, py: Python<'_>, key: &str, value: &str, ttl: Option<f64>) -> PyResult<()> {
        let json = convert::json_value(value)?;
        py.detach(|| self.cache.set_cache(key, &json, options(ttl)));
        Ok(())
    }

    // The pyo3 signature is the whole Python contract, so it stays on one line.
    #[rustfmt::skip]
    fn async_set<'py>(&self, py: Python<'py>, key: &str, value: &str, ttl: Option<f64>) -> AsyncAnswer<'py> {
        let json = convert::json_value(value)?;
        let (cache, owned) = (Arc::clone(&self.cache), key.to_owned());
        future_into_py(py, store(cache, owned, json, ttl))
    }

    fn get(&self, py: Python<'_>, key: &str) -> PyResult<Option<String>> {
        py.detach(|| self.cache.get_cache(key, false))
            .as_ref()
            .map(convert::text_of_json)
            .transpose()
            .map_err(PyErr::from)
    }

    fn async_get<'py>(&self, py: Python<'py>, key: &str) -> AsyncAnswer<'py> {
        let (cache, owned) = (Arc::clone(&self.cache), key.to_owned());
        future_into_py(py, async move {
            let stored = cache.async_get_cache(&owned, false).await;
            answer_json(stored)
        })
    }

    fn batch_get(
        &self,
        py: Python<'_>,
        keys: Vec<String>,
        local_only: bool,
    ) -> PyResult<Option<Vec<Option<String>>>> {
        let result = py.detach(|| BatchReads::batch_get_cache(&*self.cache, &keys, local_only));
        result
            .map(|values| values.iter().map(answer_text).collect())
            .transpose()
    }

    fn async_batch_get<'py>(
        &self,
        py: Python<'py>,
        keys: Vec<String>,
        local_only: bool,
        throttle_redis: bool,
    ) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            let options = BatchReadOptions {
                local_only,
                throttle_redis,
            };
            let result = BatchReads::async_batch_get_cache(&*cache, &keys, options).await;
            result
                .map(|values| values.iter().map(answer_text).collect())
                .transpose()
        })
    }

    #[staticmethod]
    fn async_batch_get_shared<'py>(
        py: Python<'py>,
        reads: Vec<(Py<NativeDual>, Vec<String>)>,
    ) -> AsyncAnswer<'py> {
        let owned = reads
            .into_iter()
            .map(|(handle, keys)| {
                let cache = Arc::clone(&handle.borrow(py).cache);
                (cache, keys)
            })
            .collect::<Vec<_>>();
        future_into_py(py, async move {
            let refs = owned
                .iter()
                .map(|(cache, keys)| (&**cache, keys.as_slice()))
                .collect::<Vec<_>>();
            let results = BatchReads::async_batch_get_cache_shared(&refs).await;
            results
                .into_iter()
                .map(|result| {
                    result
                        .map(|values| values.iter().map(answer_text).collect())
                        .transpose()
                })
                .collect::<PyResult<Vec<_>>>()
        })
    }

    fn declare_batch_get(
        &self,
        py: Python<'_>,
        keys: Vec<String>,
    ) -> PyResult<Option<Py<NativeDeclaredBatchRead>>> {
        let Some(redis) = &self.redis else {
            return Ok(None);
        };
        let Some(batch) = active_request_redis_batch(redis) else {
            return Ok(None);
        };
        let declared = BatchReads::declare_batch_get(&*self.cache, &keys, &batch)
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
        Py::new(
            py,
            NativeDeclaredBatchRead {
                cache: Arc::clone(&self.cache),
                declared: Mutex::new(Some(declared)),
            },
        )
        .map(Some)
    }

    fn set_cache_pre_call(
        &self,
        py: Python<'_>,
        key: &str,
        value: &str,
        ttl: Option<f64>,
    ) -> PyResult<Option<Py<NativeDeferred>>> {
        let json = convert::json_value(value)?;
        let result = error::backend_err(self.cache.set_cache_pre_call(key, &json, ttl))?;
        result
            .map(|result| {
                Py::new(
                    py,
                    NativeDeferred {
                        result: Mutex::new(Some(result)),
                    },
                )
            })
            .transpose()
    }

    fn delete_cache_pre_call(
        &self,
        py: Python<'_>,
        key: &str,
    ) -> PyResult<Option<Py<NativeDeferred>>> {
        let result = error::backend_err(self.cache.delete_cache_pre_call(key))?;
        result
            .map(|result| {
                Py::new(
                    py,
                    NativeDeferred {
                        result: Mutex::new(Some(result)),
                    },
                )
            })
            .transpose()
    }

    fn increment(
        &self,
        py: Python<'_>,
        key: &str,
        value: f64,
        ttl: Option<f64>,
    ) -> PyResult<String> {
        let expiry = options(ttl.or(self.memory_ttl));
        let counted =
            error::backend_err(py.detach(|| self.cache.increment_cache(key, value, expiry)))?;
        convert::number_text(counted).map_err(PyErr::from)
    }

    #[rustfmt::skip]
    fn async_increment<'py>(&self, py: Python<'py>, key: &str, value: f64, ttl: Option<f64>) -> AsyncAnswer<'py> {
        let (cache, owned, memory_ttl) = (Arc::clone(&self.cache), key.to_owned(), self.memory_ttl);
        future_into_py(py, count(cache, owned, value, ttl, memory_ttl))
    }

    fn delete(&self, py: Python<'_>, keys: Vec<String>) -> PyResult<()> {
        for key in keys {
            error::backend_err(py.detach(|| self.cache.delete_cache(&key)))?;
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
        let (cache, owned) = (Arc::clone(&self.cache), key.to_owned());
        future_into_py(py, async move {
            let ttl = error::backend_err(cache.async_get_ttl(&owned).await)?;
            text_of_ttl(ttl)
        })
    }

    fn flush(&self, py: Python<'_>) -> PyResult<()> {
        error::backend_err(py.detach(|| self.cache.flush_cache()))?;
        Ok(())
    }

    fn async_flush<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            let flushed = tokio::task::spawn_blocking(move || cache.flush_cache());
            error::backend_err(flushed.await.map_err(BackendHandleError::from)?)
        })
    }

    fn ping(&self, py: Python<'_>) -> PyResult<bool> {
        let Some(tier) = self.redis.as_ref().map(Arc::clone) else {
            return Ok(true);
        };
        error::backend_err(py.detach(|| get_runtime().block_on(tier.admin.ping(None))))
    }

    fn async_ping<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let Some(tier) = self.redis.as_ref().map(Arc::clone) else {
            return future_into_py(py, async move { Ok(true) });
        };
        future_into_py(py, async move {
            error::backend_err(tier.admin.ping(None).await)
        })
    }

    fn close(&self, py: Python<'_>) -> PyResult<()> {
        let Some(tier) = self.redis.as_ref().map(Arc::clone) else {
            return Ok(());
        };
        py.detach(|| get_runtime().block_on(close_tier(tier)))
    }

    fn async_close<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let Some(tier) = self.redis.as_ref().map(Arc::clone) else {
            return future_into_py(py, async move { Ok(()) });
        };
        future_into_py(py, close_tier(tier))
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

async fn store(cache: Arc<MemoryDual>, key: String, json: Json, ttl: Option<f64>) -> PyResult<()> {
    cache.async_set_cache(&key, &json, options(ttl)).await;
    Ok(())
}

fn answer_json(stored: Option<Json>) -> PyResult<Option<String>> {
    stored.map_or_else(
        || Ok(None),
        |value| convert::text_of_json(&value).map(Some).map_err(PyErr::from),
    )
}

fn answer_text(stored: &Option<Json>) -> PyResult<Option<String>> {
    stored.as_ref().map(convert::text_of_json).transpose()
}

async fn count(
    cache: Arc<MemoryDual>,
    key: String,
    by: f64,
    ttl: Option<f64>,
    memory_ttl: Option<f64>,
) -> PyResult<String> {
    let Some(counted) = cache
        .async_increment_cache(&key, by, options(ttl.or(memory_ttl)))
        .await
    else {
        return Err(PyErr::from(BackendHandleError::IncrementUnavailable));
    };
    convert::number_text(counted).map_err(PyErr::from)
}

async fn close_tier(tier: Arc<RedisCache>) -> PyResult<()> {
    tier.admin.disconnect().await;
    Ok(())
}

mod tests;
