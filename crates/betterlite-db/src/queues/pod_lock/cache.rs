use std::future::Future;
use std::sync::Arc;

use betterlite_redis::cache_primitives::decode::{CachedValue, validate_lua_count};
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use betterlite_redis::redis_counters::scripts::RegisteredScript;
use futures_util::future::BoxFuture;
use num_traits::ToPrimitive as _;
use redis::Value;

use super::owner::COMPARE_AND_DELETE_LOCK_SCRIPT;
use super::{LockCache, LockCacheError, LockScript};

mod tests;

// B126's RedisCache as the lease port sees it: async_set_cache with NX and EX stores the JSON string, async_get_cache returns the owner as text, and async_register_script caches the compare-and-delete Lua.
pub struct RedisLockCache {
    pub cache: Arc<RedisCache>,
}

// Each Redis call reports its own error type; the lease port keeps the operation name and the source.
trait LockCacheOperation {
    fn into_lock_error(self, operation: &'static str) -> LockCacheError;
}

impl LockCacheOperation for RedisCacheError {
    fn into_lock_error(self, operation: &'static str) -> LockCacheError {
        LockCacheError::Cache {
            operation,
            source: self,
        }
    }
}

impl LockCacheOperation for RedisCacheValuesError {
    fn into_lock_error(self, operation: &'static str) -> LockCacheError {
        LockCacheError::Values {
            operation,
            source: self,
        }
    }
}

impl LockCacheOperation for RedisAdminError {
    fn into_lock_error(self, operation: &'static str) -> LockCacheError {
        LockCacheError::Admin {
            operation,
            source: self,
        }
    }
}

fn with_cache_error<T, E>(
    operation: &'static str,
    outcome: Result<T, E>,
) -> Result<T, LockCacheError>
where
    E: LockCacheOperation,
{
    match outcome {
        Ok(value) => Ok(value),
        Err(source) => Err(source.into_lock_error(operation)),
    }
}

impl LockCache for RedisLockCache {
    #[rustfmt::skip]
    fn get_owner(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>, LockCacheError>> + Send {
        async move {
            let value = with_cache_error("async_get_cache", self.cache.async_get_cache(key).await)?;
            Ok(value.and_then(owner_bytes))
        }
    }

    // A live cache needs a running Redis: db tests fake at the LockCache seam, and the oracle fixture tests run this adapter against real Redis.
    #[cfg_attr(test, mutants::skip)]
    #[rustfmt::skip]
    fn set_owner_if_absent(&self, key: &str, owner: &str, ttl_seconds: i64) -> impl Future<Output = Result<bool, LockCacheError>> + Send {
        async move {
            let ttl = ttl_seconds.to_f64().unwrap_or_default();
            let stored = with_cache_error("async_set_cache", self.cache.values.async_set_cache(Some(key), owner, Some(ttl), true).await)?;
            Ok(stored == Some(true))
        }
    }

    #[rustfmt::skip]
    fn delete(&self, key: &str) -> impl Future<Output = Result<i64, LockCacheError>> + Send {
        async move {
            with_cache_error("async_delete_cache", self.cache.admin.async_delete_cache(key).await)
        }
    }

    // The registered handle is only exercised by a script run, which needs the running Redis of the oracle fixture tests.
    #[cfg_attr(test, mutants::skip)]
    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        let script = self
            .cache
            .counters
            .async_register_script(COMPARE_AND_DELETE_LOCK_SCRIPT);
        Some(Arc::new(RedisLockScript {
            cache: Arc::clone(&self.cache),
            script,
        }))
    }
}

// async_set_cache stored json.dumps(pod_id), so the script's ARGV[1] must carry the same quoted text.
struct RedisLockScript {
    cache: Arc<RedisCache>,
    script: RegisteredScript,
}

impl LockScript for RedisLockScript {
    fn run<'a>(
        &'a self,
        key: &'a str,
        owner: &'a str,
    ) -> BoxFuture<'a, Result<bool, LockCacheError>> {
        Box::pin(async move {
            let owner_json = serde_json::to_string(owner)?;
            let keys = [key.to_owned()];
            let args = [RedisScalar::Text(owner_json)];
            deleted_one(with_cache_error(
                "run_script",
                self.cache.run_script(&self.script, &keys, &args).await,
            )?)
        })
    }
}

fn deleted_one(reply: Value) -> Result<bool, LockCacheError> {
    match reply {
        Value::Int(count) => Ok(count == 1),
        Value::BulkString(bytes) => Ok(validate_lua_count(&bytes)? == 1),
        Value::Nil => Ok(false),
        _ => Err(LockCacheError::UnexpectedReply {
            operation: "run_script",
        }),
    }
}

// async_get_cache decodes the stored JSON; only a string can be the owner, and a non-string never equals a UUID.
fn owner_bytes(value: CachedValue) -> Option<Vec<u8>> {
    match value {
        CachedValue::Text(text) => Some(text.into_bytes()),
        CachedValue::Null
        | CachedValue::Boolean(_)
        | CachedValue::Number(_)
        | CachedValue::Array(_)
        | CachedValue::Object(_) => None,
    }
}
