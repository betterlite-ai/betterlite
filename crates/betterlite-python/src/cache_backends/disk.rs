use std::sync::Arc;

use betterlite_cache::coordinator::AsyncCacheProtocol as _;
use betterlite_cache::disk::DiskCache;
use betterlite_context::local_ttl_cache::CacheValue;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;

use super::convert;
use super::error;
use super::settings::DiskSettings;

type AsyncAnswer<'py> = PyResult<Bound<'py, PyAny>>;

// DiskCache over Rust's sqlite store; a directory the store cannot open fails at construction.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeDisk {
    pub(crate) cache: Arc<DiskCache>,
}

#[pymethods]
impl NativeDisk {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let parsed: DiskSettings = convert::parse_settings(settings)?;
        let clock = Arc::new(convert::wall_clock);
        let cache = error::backend_err(DiskCache::open(parsed.disk_cache_dir.as_deref(), clock))?;
        Ok(Self {
            cache: Arc::new(cache),
        })
    }

    fn set(&self, key: &str, value: &str, ttl: Option<f64>) -> PyResult<()> {
        let cache_value = convert::cache_value(value)?;
        error::backend_err(self.cache.set_cache(key, &cache_value, ttl))?;
        Ok(())
    }

    // The pyo3 signature is the whole Python contract, so it stays on one line.
    #[rustfmt::skip]
    fn async_set<'py>(&self, py: Python<'py>, key: &str, value: &str, ttl: Option<f64>) -> AsyncAnswer<'py> {
        let cache_value = convert::cache_value(value)?;
        let (cache, owned) = (Arc::clone(&self.cache), key.to_owned());
        future_into_py(py, async move {
            error::backend_err(cache.async_set_cache(&owned, cache_value, ttl).await)
        })
    }

    fn get(&self, key: &str) -> PyResult<Option<String>> {
        let stored = error::backend_err(self.cache.get_cache(key))?;
        stored
            .as_ref()
            .map(convert::text_of_value)
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

    // DiskCache.increment_cache takes whole numbers and answers one, as LiteLLM's int-typed method does.
    fn increment(&self, key: &str, value: i64, ttl: Option<f64>) -> PyResult<String> {
        error::backend_err(self.cache.increment_cache(key, value, ttl))
            .map(|counted| counted.to_string())
    }

    #[rustfmt::skip]
    fn async_increment<'py>(&self, py: Python<'py>, key: &str, value: i64, ttl: Option<f64>) -> AsyncAnswer<'py> {
        let (cache, owned) = (Arc::clone(&self.cache), key.to_owned());
        future_into_py(py, count(cache, owned, value, ttl))
    }

    fn delete(&self, keys: Vec<String>) -> PyResult<()> {
        for key in keys {
            error::backend_err(self.cache.delete_cache(&key))?;
        }
        Ok(())
    }

    fn async_delete<'py>(&self, py: Python<'py>, keys: Vec<String>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            for key in &keys {
                error::backend_err(cache.async_delete_cache(key).await)?;
            }
            Ok(())
        })
    }

    // LiteLLM's DiskCache has no ttl read; the lookup keeps a broken store's failure visible.
    fn ttl(&self, key: &str) -> PyResult<Option<String>> {
        let _stored = error::backend_err(self.cache.get_cache(key))?;
        Ok(None)
    }

    fn async_ttl<'py>(&self, py: Python<'py>, key: &str) -> AsyncAnswer<'py> {
        let (cache, owned) = (Arc::clone(&self.cache), key.to_owned());
        future_into_py(py, async move {
            let _stored = error::backend_err(cache.async_get_cache(&owned).await)?;
            answer_text(None)
        })
    }

    fn flush(&self) -> PyResult<()> {
        error::backend_err(self.cache.flush_cache())?;
        Ok(())
    }

    fn async_flush<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            error::backend_err(cache.async_flush_cache().await)?;
            Ok(())
        })
    }

    fn ping(&self) -> PyResult<bool> {
        let _stored = error::backend_err(self.cache.get_cache(""))?;
        Ok(true)
    }

    fn async_ping<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            let _stored = error::backend_err(cache.async_get_cache("").await)?;
            Ok(true)
        })
    }

    #[expect(
        clippy::missing_const_for_fn,
        clippy::unnecessary_wraps,
        clippy::unused_self,
        reason = "DiskCache.disconnect is a no-op at the pin"
    )]
    // ast-grep-ignore: no-one-line-functions - DiskCache.disconnect is a no-op at the pin
    fn close(&self) -> PyResult<()> {
        Ok(())
    }

    // ast-grep-ignore: no-padded-one-line-functions - the awaitable of an immediate answer is the whole behavior
    fn async_close<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let closed = self.close();
        future_into_py(py, async move { closed })
    }
}

fn answer_text(stored: Option<CacheValue>) -> PyResult<Option<String>> {
    stored.map_or_else(
        || Ok(None),
        |value| {
            convert::text_of_value(&value)
                .map(Some)
                .map_err(PyErr::from)
        },
    )
}

async fn count(cache: Arc<DiskCache>, key: String, by: i64, ttl: Option<f64>) -> PyResult<String> {
    error::backend_err(cache.async_increment(&key, by, ttl).await)
        .map(|counted| counted.to_string())
}

mod tests;
