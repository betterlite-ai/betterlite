use std::sync::Arc;

use betterlite_cache::coordinator::AsyncCacheProtocol;
use betterlite_cache::disk::DiskCache;
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
        let cache = error::to_py(DiskCache::open(parsed.disk_cache_dir.as_deref(), clock))?;
        Ok(Self {
            cache: Arc::new(cache),
        })
    }

    fn set(&self, key: &str, value: &str, ttl: Option<f64>) -> PyResult<()> {
        let cache_value = convert::cache_value(value)?;
        error::to_py(self.cache.set_cache(key, &cache_value, ttl))?;
        Ok(())
    }

    fn async_set<'py>(
        &self,
        py: Python<'py>,
        key: &str,
        value: &str,
        ttl: Option<f64>,
    ) -> AsyncAnswer<'py> {
        let cache_value = convert::cache_value(value)?;
        let cache = Arc::clone(&self.cache);
        let owned = key.to_owned();
        future_into_py(py, async move {
            error::to_py(cache.async_set_cache(&owned, cache_value, ttl).await)?;
            Ok(())
        })
    }

    fn get(&self, key: &str) -> PyResult<Option<String>> {
        let stored = error::to_py(self.cache.get_cache(key))?;
        stored
            .as_ref()
            .map(convert::text_of_value)
            .transpose()
            .map_err(PyErr::from)
    }

    fn async_get<'py>(&self, py: Python<'py>, key: &str) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        let owned = key.to_owned();
        future_into_py(py, async move {
            let stored = error::to_py(cache.async_get_cache(&owned).await)?;
            stored
                .as_ref()
                .map(convert::text_of_value)
                .transpose()
                .map_err(PyErr::from)
        })
    }

    // DiskCache.increment_cache takes whole numbers and answers one, as LiteLLM's int-typed method does.
    fn increment(&self, key: &str, value: f64, ttl: Option<f64>) -> PyResult<String> {
        let whole = convert::whole_increment(value)?;
        let counted = error::to_py(self.cache.increment_cache(key, whole, ttl))?;
        convert::whole_text(counted).map_err(PyErr::from)
    }

    fn async_increment<'py>(
        &self,
        py: Python<'py>,
        key: &str,
        value: f64,
        ttl: Option<f64>,
    ) -> AsyncAnswer<'py> {
        let whole = convert::whole_increment(value)?;
        let cache = Arc::clone(&self.cache);
        let owned = key.to_owned();
        future_into_py(py, async move {
            let counted = error::to_py(cache.async_increment(&owned, whole, ttl).await)?;
            convert::whole_text(counted).map_err(PyErr::from)
        })
    }

    fn delete(&self, keys: Vec<String>) -> PyResult<()> {
        for key in &keys {
            error::to_py(self.cache.delete_cache(key))?;
        }
        Ok(())
    }

    fn async_delete<'py>(&self, py: Python<'py>, keys: Vec<String>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            for key in &keys {
                error::to_py(cache.async_delete_cache(key).await)?;
            }
            Ok(())
        })
    }

    // LiteLLM's DiskCache has no ttl read; the lookup keeps a broken store's failure visible.
    fn ttl(&self, key: &str) -> PyResult<Option<String>> {
        let _stored = error::to_py(self.cache.get_cache(key))?;
        Ok(None)
    }

    fn async_ttl<'py>(&self, py: Python<'py>, key: &str) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        let owned = key.to_owned();
        future_into_py(py, async move {
            let _stored = error::to_py(cache.get_cache(&owned))?;
            Ok(None)
        })
    }

    fn flush(&self) -> PyResult<()> {
        error::to_py(self.cache.flush_cache())?;
        Ok(())
    }

    fn async_flush<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            error::to_py(cache.async_flush_cache().await)?;
            Ok(())
        })
    }

    fn ping(&self) -> PyResult<bool> {
        let _stored = error::to_py(self.cache.get_cache(""))?;
        Ok(true)
    }

    fn async_ping<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let cache = Arc::clone(&self.cache);
        future_into_py(py, async move {
            let _stored = error::to_py(cache.get_cache(""))?;
            Ok(true)
        })
    }

    // DiskCache.disconnect is a no-op at the pin.
    fn close(&self) -> PyResult<()> {
        Ok(())
    }

    fn async_close<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        future_into_py(py, async move { Ok(()) })
    }
}
