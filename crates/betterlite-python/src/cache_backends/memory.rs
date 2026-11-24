use std::sync::Arc;

use betterlite_cache::backend::memory::MemoryCacheOps as _;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::DEFAULT_MAX_SIZE_IN_MEMORY;
use betterlite_cache::dual_cache::MemoryMaxSize as _;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;

use super::convert;
use super::error::{self, BackendHandleError};
use super::settings::MemorySettings;

// Every async entry point hands Python the same awaitable of JSON text.
type AsyncAnswer<'py> = PyResult<Bound<'py, PyAny>>;

const DEFAULT_MAX_SIZE_PER_ITEM_KIB: i64 = 1024;

// InMemoryCache over the shared in-memory tier; the Python class passes the effective ttl on each write.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeMemory {
    pub(crate) ops: Arc<LocalMemoryOps>,
}

#[pymethods]
impl NativeMemory {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let parsed: MemorySettings = convert::parse_settings(settings)?;
        let clock = Arc::new(convert::wall_clock);
        let item_kib = parsed
            .max_size_per_item
            .unwrap_or(DEFAULT_MAX_SIZE_PER_ITEM_KIB);
        let ops = error::to_py(LocalMemoryOps::new(item_kib, clock))?;
        ops.set_max_size(
            parsed
                .max_size_in_memory
                .unwrap_or(DEFAULT_MAX_SIZE_IN_MEMORY),
        );
        Ok(Self { ops: Arc::new(ops) })
    }

    fn set(&self, key: &str, value: &str, ttl: Option<f64>) -> PyResult<()> {
        let cache_value = convert::cache_value(value)?;
        error::to_py(self.ops.set_cache(key, cache_value, ttl))?;
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
        let ops = Arc::clone(&self.ops);
        let owned = key.to_owned();
        future_into_py(py, async move {
            error::to_py(ops.set_cache(&owned, cache_value, ttl))?;
            Ok(())
        })
    }

    fn get(&self, key: &str) -> PyResult<Option<String>> {
        let stored = error::to_py(self.ops.get_cache(key))?;
        stored
            .as_ref()
            .map(convert::text_of_value)
            .transpose()
            .map_err(PyErr::from)
    }

    fn async_get<'py>(&self, py: Python<'py>, key: &str) -> AsyncAnswer<'py> {
        let ops = Arc::clone(&self.ops);
        let owned = key.to_owned();
        future_into_py(py, async move {
            let stored = error::to_py(ops.get_cache(&owned))?;
            stored
                .as_ref()
                .map(convert::text_of_value)
                .transpose()
                .map_err(PyErr::from)
        })
    }

    fn increment(&self, key: &str, value: f64, ttl: Option<f64>) -> PyResult<String> {
        let counted = error::to_py(self.ops.increment_cache(key, value, ttl))?;
        convert::number_text(counted).map_err(PyErr::from)
    }

    fn async_increment<'py>(
        &self,
        py: Python<'py>,
        key: &str,
        value: f64,
        ttl: Option<f64>,
    ) -> AsyncAnswer<'py> {
        let ops = Arc::clone(&self.ops);
        let owned = key.to_owned();
        future_into_py(py, async move {
            let counted = error::to_py(ops.increment_cache(&owned, value, ttl))?;
            convert::number_text(counted).map_err(PyErr::from)
        })
    }

    fn delete(&self, keys: Vec<String>) -> PyResult<()> {
        for key in &keys {
            error::to_py(self.ops.delete_cache(key))?;
        }
        Ok(())
    }

    fn async_delete<'py>(&self, py: Python<'py>, keys: Vec<String>) -> AsyncAnswer<'py> {
        let ops = Arc::clone(&self.ops);
        future_into_py(py, async move {
            for key in &keys {
                error::to_py(ops.delete_cache(key))?;
            }
            Ok(())
        })
    }

    fn ttl(&self, key: &str) -> PyResult<Option<String>> {
        let entries = error::to_py(self.ops.ttl_entries())?;
        entries
            .into_iter()
            .find(|(name, _expiry)| name == key)
            .map(|(_name, expiry)| convert::number_text(expiry))
            .transpose()
            .map_err(PyErr::from)
    }

    // ast-grep-ignore: no-padded-one-line-functions - the sync and async entry points share this read
    fn async_ttl<'py>(&self, py: Python<'py>, key: &str) -> AsyncAnswer<'py> {
        let ops = Arc::clone(&self.ops);
        let owned = key.to_owned();
        future_into_py(py, async move {
            let entries = error::to_py(ops.ttl_entries())?;
            entries
                .into_iter()
                .find(|(name, _expiry)| name == owned)
                .map(|(_name, expiry)| convert::number_text(expiry))
                .transpose()
                .map_err(PyErr::from)
        })
    }

    fn flush(&self) -> PyResult<()> {
        error::to_py(self.ops.flush_cache())?;
        Ok(())
    }

    fn async_flush<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let ops = Arc::clone(&self.ops);
        future_into_py(py, async move {
            error::to_py(ops.flush_cache())?;
            Ok(())
        })
    }

    fn ping(&self) -> PyResult<bool> {
        let _entries = error::to_py(self.ops.ttl_entries())?;
        Ok(true)
    }

    fn async_ping<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let ops = Arc::clone(&self.ops);
        future_into_py(py, async move {
            let _entries = error::to_py(ops.ttl_entries())?;
            Ok(true)
        })
    }

    // InMemoryCache.disconnect is a no-op at the pin, and so is the native tier's.
    fn close(&self) -> PyResult<()> {
        Ok(())
    }

    fn async_close<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        future_into_py(py, async move { Ok(()) })
    }
}
