use std::sync::Arc;

use betterlite_cache::backend::memory::MemoryCacheOps as _;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::DEFAULT_MAX_SIZE_IN_MEMORY;
use betterlite_cache::dual_cache::MemoryMaxSize as _;
use betterlite_context::local_ttl_cache::CacheValue;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;

use super::convert;
use super::error;
use super::settings::MemorySettings;

// Every async entry point hands Python the same awaitable of JSON text.
type AsyncAnswer<'py> = PyResult<Bound<'py, PyAny>>;

const DEFAULT_MAX_SIZE_PER_ITEM_KIB: i64 = 1024;

// InMemoryCache over the shared in-memory tier; the Python class passes the effective ttl on each write.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeMemory {
    pub(crate) ops: Arc<LocalMemoryOps>,
    pub(crate) default_ttl: Option<f64>,
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
        let ops = error::backend_err(LocalMemoryOps::new(item_kib, clock))?;
        ops.set_max_size(
            parsed
                .max_size_in_memory
                .unwrap_or(DEFAULT_MAX_SIZE_IN_MEMORY),
        );
        Ok(Self {
            ops: Arc::new(ops),
            default_ttl: parsed.default_ttl,
        })
    }

    fn set(&self, key: &str, value: &str, ttl: Option<f64>) -> PyResult<()> {
        let cache_value = convert::cache_value(value)?;
        error::backend_err(
            self.ops
                .set_cache(key, cache_value, ttl.or(self.default_ttl)),
        )?;
        Ok(())
    }

    // The pyo3 signature is the whole Python contract, so it stays on one line.
    #[rustfmt::skip]
    fn async_set<'py>(&self, py: Python<'py>, key: &str, value: &str, ttl: Option<f64>) -> AsyncAnswer<'py> {
        let cache_value = convert::cache_value(value)?;
        let (ops, owned, expiry) = (Arc::clone(&self.ops), key.to_owned(), ttl.or(self.default_ttl));
        future_into_py(py, async move { set_value(&ops, &owned, cache_value, expiry) })
    }

    fn get(&self, key: &str) -> PyResult<Option<String>> {
        let stored = error::backend_err(self.ops.get_cache(key))?;
        stored
            .as_ref()
            .map(convert::text_of_value)
            .transpose()
            .map_err(PyErr::from)
    }

    fn async_get<'py>(&self, py: Python<'py>, key: &str) -> AsyncAnswer<'py> {
        let (ops, owned) = (Arc::clone(&self.ops), key.to_owned());
        future_into_py(py, async move {
            let stored = error::backend_err(ops.get_cache(&owned))?;
            answer_text(stored)
        })
    }

    fn increment(&self, key: &str, value: f64, ttl: Option<f64>) -> PyResult<String> {
        let counted = error::backend_err(self.ops.increment_cache(
            key,
            value,
            ttl.or(self.default_ttl),
        ))?;
        convert::number_text(counted).map_err(PyErr::from)
    }

    #[rustfmt::skip]
    fn async_increment<'py>(&self, py: Python<'py>, key: &str, value: f64, ttl: Option<f64>) -> AsyncAnswer<'py> {
        let (ops, owned, expiry) = (Arc::clone(&self.ops), key.to_owned(), ttl.or(self.default_ttl));
        future_into_py(py, async move {
            let counted = error::backend_err(ops.increment_cache(&owned, value, expiry))?;
            convert::number_text(counted).map_err(PyErr::from)
        })
    }

    fn delete(&self, keys: Vec<String>) -> PyResult<()> {
        for key in keys {
            error::backend_err(self.ops.delete_cache(&key))?;
        }
        Ok(())
    }

    fn async_delete<'py>(&self, py: Python<'py>, keys: Vec<String>) -> AsyncAnswer<'py> {
        let ops = Arc::clone(&self.ops);
        future_into_py(py, async move {
            for key in &keys {
                error::backend_err(ops.delete_cache(key))?;
            }
            Ok(())
        })
    }

    fn ttl(&self, key: &str) -> PyResult<Option<String>> {
        let entries = error::backend_err(self.ops.ttl_entries())?;
        match entries.into_iter().find(|(name, _expiry)| name == key) {
            Some((_name, expiry)) => convert::number_text(expiry).map(Some).map_err(PyErr::from),
            None => Ok(None),
        }
    }

    fn async_ttl<'py>(&self, py: Python<'py>, key: &str) -> AsyncAnswer<'py> {
        let (ops, owned) = (Arc::clone(&self.ops), key.to_owned());
        future_into_py(py, async move {
            let entries = error::backend_err(ops.ttl_entries())?;
            remaining_ttl(entries, &owned)
        })
    }

    fn flush(&self) -> PyResult<()> {
        error::backend_err(self.ops.flush_cache())?;
        Ok(())
    }

    fn async_flush<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let ops = Arc::clone(&self.ops);
        future_into_py(py, async move {
            error::backend_err(ops.flush_cache())?;
            Ok(())
        })
    }

    fn ping(&self) -> PyResult<bool> {
        let _entries = error::backend_err(self.ops.ttl_entries())?;
        Ok(true)
    }

    fn async_ping<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let ops = Arc::clone(&self.ops);
        future_into_py(py, async move {
            let _entries = error::backend_err(ops.ttl_entries())?;
            Ok(true)
        })
    }

    #[expect(
        clippy::missing_const_for_fn,
        clippy::unnecessary_wraps,
        clippy::unused_self,
        reason = "InMemoryCache.disconnect and the native tier are no-ops at the pin"
    )]
    // ast-grep-ignore: no-one-line-functions - InMemoryCache.disconnect and the native tier are no-ops at the pin
    fn close(&self) -> PyResult<()> {
        Ok(())
    }

    // ast-grep-ignore: no-padded-one-line-functions - the awaitable of an immediate answer is the whole behavior
    fn async_close<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let closed = self.close();
        future_into_py(py, async move { closed })
    }
}

fn set_value(ops: &LocalMemoryOps, key: &str, value: CacheValue, ttl: Option<f64>) -> PyResult<()> {
    error::backend_err(ops.set_cache(key, value, ttl))?;
    Ok(())
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

fn remaining_ttl(entries: Vec<(String, f64)>, key: &str) -> PyResult<Option<String>> {
    match entries.into_iter().find(|(name, _expiry)| name == key) {
        Some((_name, expiry)) => convert::number_text(expiry).map(Some).map_err(PyErr::from),
        None => Ok(None),
    }
}

mod tests;
