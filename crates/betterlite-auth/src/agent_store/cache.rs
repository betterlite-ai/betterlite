use async_trait::async_trait;
use betterlite_models::decisions::Json;

use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::{AuthCache, WriteOptions, WriteTtl};

mod tests;

// the part of LiteLLM's UserApiKeyCache the identity store reads and writes
#[async_trait]
pub trait ManagementObjectCache: Send + Sync {
    async fn read(&self, key: &str) -> Option<Json>;
    fn management_ttl(&self) -> f64;
    async fn write(&self, key: &str, value: &Json, ttl: f64);
}

// LiteLLM caches the plain agent_id string under the binding key, with the management-object TTL
#[async_trait]
impl<T> ManagementObjectCache for AuthCache<T>
where
    T: AuthCacheTier + 'static,
{
    async fn read(&self, key: &str) -> Option<Json> {
        self.async_get(key, false).await
    }

    fn management_ttl(&self) -> f64 {
        self.management_object_ttl()
    }

    async fn write(&self, key: &str, value: &Json, ttl: f64) {
        let options = WriteOptions {
            ttl: Some(WriteTtl { seconds: ttl }),
            local_only: false,
            refresh_ttl: false,
        };
        // DualCache logs a failed write and continues, so the binding lookup still answers with its row
        drop(self.async_set(Some(key), value, options).await);
    }
}
