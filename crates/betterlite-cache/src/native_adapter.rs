use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;

use crate::backend::{
    BASE_DEFAULT_TTL_SECONDS, CacheBackend, CacheResult, ConnectionStatus, ConnectionTest, Ttl,
};

pub const DEFAULT_HANDLE_TTL_SECONDS: f64 = 600.0;
pub const DEFAULT_MEMORY_CAPACITY: usize = 200;
// 4 MiB, the limit the legacy facade shares with the v2 factories
pub const DEFAULT_MAX_ENTRY_BYTES: usize = 0x0040_0000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MemoryHandleOptions {
    pub ttl: f64,
    pub capacity: usize,
    pub max_entry_bytes: usize,
}

impl Default for MemoryHandleOptions {
    fn default() -> Self {
        Self {
            ttl: DEFAULT_HANDLE_TTL_SECONDS,
            capacity: DEFAULT_MEMORY_CAPACITY,
            max_entry_bytes: DEFAULT_MAX_ENTRY_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RedisHandleOptions {
    pub ttl: f64,
    pub max_entry_bytes: usize,
}

impl Default for RedisHandleOptions {
    fn default() -> Self {
        Self {
            ttl: DEFAULT_HANDLE_TTL_SECONDS,
            max_entry_bytes: DEFAULT_MAX_ENTRY_BYTES,
        }
    }
}

// the storage contract LiteLLM's rust_bridge._native NativeCacheHandle declares for the v2 factories
#[async_trait]
pub trait NativeCacheHandle: Send + Sync {
    fn get(&self, key: &str) -> CacheResult<Option<CacheValue>>;

    fn set(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()>;

    async fn async_get(&self, key: &str) -> CacheResult<Option<CacheValue>>;

    async fn async_set(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()>;

    async fn async_set_many(&self, entries: Vec<(String, CacheValue)>, ttl: Ttl)
    -> CacheResult<()>;

    fn flush(&self) -> CacheResult<()>;

    async fn async_flush(&self) -> CacheResult<()>;

    async fn ping(&self) -> CacheResult<bool>;

    async fn disconnect(&self) -> CacheResult<()>;

    async fn delete(&self, keys: &[String]) -> CacheResult<()>;
}

// LiteLLM imports NativeCacheHandle inside each factory, so the constructor is the host's to provide
pub trait NativeHandleFactory {
    type Error: Error + Send + Sync + 'static;

    fn memory_handle(
        &self,
        options: MemoryHandleOptions,
    ) -> Result<Arc<dyn NativeCacheHandle>, Self::Error>;

    fn redis_handle(
        &self,
        url: &str,
        namespace: &str,
        options: RedisHandleOptions,
    ) -> Result<Arc<dyn NativeCacheHandle>, Self::Error>;
}

// BaseCache passes no kwargs the native handle reads, so the adapter drops them and forwards the typed ttl
pub struct NativeBackend {
    pub handle: Arc<dyn NativeCacheHandle>,
}

#[async_trait]
pub trait NativeCacheLifecycle: Send + Sync {
    async fn ping(&self) -> CacheResult<bool>;

    async fn async_flush_cache(&self) -> CacheResult<()>;

    async fn delete_cache_keys(&self, keys: &[String]) -> CacheResult<()>;
}

#[async_trait]
impl NativeCacheLifecycle for NativeBackend {
    async fn ping(&self) -> CacheResult<bool> {
        self.handle.ping().await
    }

    async fn async_flush_cache(&self) -> CacheResult<()> {
        self.handle.async_flush().await
    }

    async fn delete_cache_keys(&self, keys: &[String]) -> CacheResult<()> {
        self.handle.delete(keys).await
    }
}

#[async_trait]
impl CacheBackend for NativeBackend {
    fn default_ttl(&self) -> i64 {
        BASE_DEFAULT_TTL_SECONDS
    }

    fn get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        self.handle.get(key)
    }

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.handle.set(key, value, ttl)
    }

    fn flush_cache(&self) -> CacheResult<()> {
        self.handle.flush()
    }

    async fn async_get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        self.handle.async_get(key).await
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.handle.async_set(key, value, ttl).await
    }

    async fn async_set_cache_pipeline(
        &self,
        entries: Vec<(String, CacheValue)>,
        ttl: Ttl,
    ) -> CacheResult<()> {
        self.handle.async_set_many(entries, ttl).await
    }

    async fn batch_cache_write(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.handle.async_set(key, value, ttl).await
    }

    async fn disconnect(&self) -> CacheResult<()> {
        self.handle.disconnect().await
    }

    async fn test_connection(&self) -> CacheResult<ConnectionTest> {
        let (status, message) = if self.handle.ping().await? {
            (
                ConnectionStatus::Success,
                "the native cache answered its ping",
            )
        } else {
            (
                ConnectionStatus::Failed,
                "the native cache did not answer its ping",
            )
        };
        Ok(ConnectionTest {
            status,
            message: message.to_owned(),
            error: None,
        })
    }
}

mod tests;
