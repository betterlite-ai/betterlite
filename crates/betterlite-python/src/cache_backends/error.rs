use betterlite_cache::backend::BackendError;
use betterlite_cache::disk::DiskError;
use betterlite_cache::dual_cache::DualCacheError;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use betterlite_redis::redis_counters::RedisCountersError;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::{PyErr, PyResult};
use tokio::task::JoinError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum BackendHandleError {
    #[error("the backend settings are not JSON of the expected shape: {source}")]
    Settings {
        #[source]
        source: serde_json::Error,
    },
    #[error("the cache value is not JSON: {source}")]
    Value {
        #[source]
        source: serde_json::Error,
    },
    #[error("the cache value has no JSON text: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the number {value} has no JSON form")]
    UnrepresentableNumber { value: f64 },
    #[error("the increment of {value} is not a whole number")]
    NotWhole { value: f64 },
    #[error("the increment result is not available")]
    IncrementUnavailable,
    #[error("the in-memory cache tier failed")]
    Memory {
        #[from]
        source: BackendError,
    },
    #[error("the disk cache failed")]
    Disk {
        #[from]
        source: DiskError,
    },
    #[error("the Redis cache failed")]
    Redis {
        #[from]
        source: RedisCacheError,
    },
    #[error("the Redis value call failed")]
    RedisValues {
        #[from]
        source: RedisCacheValuesError,
    },
    #[error("the Redis admin call failed")]
    RedisAdmin {
        #[from]
        source: RedisAdminError,
    },
    #[error("the Redis counter call failed")]
    RedisCounters {
        #[from]
        source: RedisCountersError,
    },
    #[error("the dual cache failed")]
    Dual {
        #[from]
        source: DualCacheError,
    },
    #[error("the blocking cache task failed")]
    Join {
        #[from]
        source: JoinError,
    },
}

impl BackendHandleError {
    pub(crate) const fn is_value_error(&self) -> bool {
        matches!(
            self,
            Self::Settings { .. } | Self::Value { .. } | Self::NotWhole { .. }
        )
    }
}

impl From<BackendHandleError> for PyErr {
    fn from(error: BackendHandleError) -> Self {
        if error.is_value_error() {
            return PyValueError::new_err(error.to_string());
        }
        PyRuntimeError::new_err(error.to_string())
    }
}

// Every backend call converts its own error type through BackendHandleError into the Python exception.
pub(crate) fn backend_err<Value, Failure>(result: Result<Value, Failure>) -> PyResult<Value>
where
    BackendHandleError: From<Failure>,
{
    result
        .map_err(BackendHandleError::from)
        .map_err(PyErr::from)
}

mod tests;
