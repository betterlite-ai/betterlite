pub mod memory;

#[path = "backend/tests.rs"]
mod tests;

use std::num::TryFromIntError;
use std::time::TryFromFloatSecsError;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::{CacheError, CacheValue};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::pipeline::RedisPipelineIncrement;

pub const BASE_DEFAULT_TTL_SECONDS: i64 = 60;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("the {operation} operation is not implemented by this cache backend")]
    Unsupported { operation: &'static str },
    #[error("the cache operation failed")]
    Cache {
        #[from]
        source: CacheError,
    },
    #[error("the cached value under {key} is not a number")]
    NotNumeric { key: String },
    #[error("the increment under {key} left the finite range")]
    IncrementOutOfRange { key: String },
    #[error("the expiry of {key} left the finite range")]
    InvalidExpiry { key: String },
    #[error("the cache default ttl {value} is not a finite duration")]
    InvalidDefaultDuration {
        value: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the cache default ttl {value} cannot be written as whole seconds")]
    InvalidDefaultSeconds {
        value: f64,
        #[source]
        source: TryFromIntError,
    },
    #[error("the cache default ttl {value} cannot be represented")]
    InvalidDefaultTtl { value: f64 },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    Success,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConnectionTest {
    pub status: ConnectionStatus,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[async_trait]
pub trait CacheBackend: Send + Sync {
    fn default_ttl(&self) -> i64;

    fn get_ttl(&self, ttl: Option<i64>) -> i64 {
        ttl.unwrap_or_else(|| self.default_ttl())
    }

    fn get_cache(&self, _key: &str) -> Result<Option<CacheValue>, BackendError> {
        Err(BackendError::Unsupported {
            operation: "get_cache",
        })
    }

    fn set_cache(
        &self,
        _key: &str,
        _value: CacheValue,
        _ttl: Option<f64>,
    ) -> Result<(), BackendError> {
        Err(BackendError::Unsupported {
            operation: "set_cache",
        })
    }

    fn delete_cache(&self, _key: &str) -> Result<(), BackendError> {
        Err(BackendError::Unsupported {
            operation: "delete_cache",
        })
    }

    fn flush_cache(&self) -> Result<(), BackendError> {
        Err(BackendError::Unsupported {
            operation: "flush_cache",
        })
    }

    async fn async_get_cache(&self, _key: &str) -> Result<Option<CacheValue>, BackendError> {
        Err(BackendError::Unsupported {
            operation: "async_get_cache",
        })
    }

    async fn async_set_cache(
        &self,
        _key: &str,
        _value: CacheValue,
        _ttl: Option<f64>,
    ) -> Result<(), BackendError> {
        Err(BackendError::Unsupported {
            operation: "async_set_cache",
        })
    }

    async fn async_set_cache_pipeline(
        &self,
        _entries: Vec<(String, CacheValue)>,
        _ttl: Option<f64>,
    ) -> Result<(), BackendError> {
        Err(BackendError::Unsupported {
            operation: "async_set_cache_pipeline",
        })
    }

    async fn async_batch_get_cache(
        &self,
        _keys: &[String],
    ) -> Result<Vec<Option<CacheValue>>, BackendError> {
        Err(BackendError::Unsupported {
            operation: "async_batch_get_cache",
        })
    }

    async fn async_increment(
        &self,
        _key: &str,
        _value: f64,
        _ttl: Option<f64>,
    ) -> Result<f64, BackendError> {
        Err(BackendError::Unsupported {
            operation: "async_increment",
        })
    }

    async fn async_increment_pipeline(
        &self,
        _increments: Vec<RedisPipelineIncrement>,
    ) -> Result<Vec<f64>, BackendError> {
        Err(BackendError::Unsupported {
            operation: "async_increment_pipeline",
        })
    }

    async fn async_get_ttl(&self, _key: &str) -> Result<Option<f64>, BackendError> {
        Err(BackendError::Unsupported {
            operation: "async_get_ttl",
        })
    }

    async fn async_get_oldest_n_keys(&self, _limit: usize) -> Result<Vec<String>, BackendError> {
        Err(BackendError::Unsupported {
            operation: "async_get_oldest_n_keys",
        })
    }

    async fn batch_cache_write(
        &self,
        _key: &str,
        _value: CacheValue,
        _ttl: Option<f64>,
    ) -> Result<(), BackendError> {
        Err(BackendError::Unsupported {
            operation: "batch_cache_write",
        })
    }

    async fn disconnect(&self) -> Result<(), BackendError> {
        Err(BackendError::Unsupported {
            operation: "disconnect",
        })
    }

    async fn test_connection(&self) -> Result<ConnectionTest, BackendError> {
        Err(BackendError::Unsupported {
            operation: "test_connection",
        })
    }
}
