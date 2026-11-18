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
    async fn get_owner(&self, key: &str) -> Result<Option<Vec<u8>>, LockCacheError> {
        let value = with_cache_error("async_get_cache", self.cache.async_get_cache(key).await)?;
        Ok(value.and_then(owner_bytes))
    }

    // A live cache needs a running Redis, which the db tests fake at the LockCache seam.
    #[cfg_attr(test, mutants::skip)]
    async fn set_owner_if_absent(
        &self,
        key: &str,
        owner: &str,
        ttl_seconds: i64,
    ) -> Result<bool, LockCacheError> {
        let ttl = ttl_seconds.to_f64().unwrap_or_default();
        let stored = with_cache_error(
            "async_set_cache",
            self.cache
                .values
                .async_set_cache(Some(key), owner, Some(ttl), true)
                .await,
        )?;
        Ok(stored == Some(true))
    }

    async fn delete(&self, key: &str) -> Result<i64, LockCacheError> {
        with_cache_error(
            "async_delete_cache",
            self.cache.admin.async_delete_cache(key).await,
        )
    }

    // Registering the handle needs a running Redis, which the db tests fake at the LockCache seam.
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

// ARGV[1] is the JSON text async_set_cache stored, which is what the Lua GET equality compares against.
struct RedisLockScript {
    cache: Arc<RedisCache>,
    script: RegisteredScript,
}

impl LockScript for RedisLockScript {
    // The handle needs a running Redis, which the db tests fake at the LockCache seam.
    #[cfg_attr(test, mutants::skip)]
    fn run<'lease>(
        &'lease self,
        key: &'lease str,
        owner_json: &'lease str,
    ) -> BoxFuture<'lease, Result<bool, LockCacheError>> {
        Box::pin(async move {
            let keys = [key.to_owned()];
            let args = [RedisScalar::Text(owner_json.to_owned())];
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
        Value::Array(_)
        | Value::SimpleString(_)
        | Value::Okay
        | Value::Map(_)
        | Value::Attribute { .. }
        | Value::Set(_)
        | Value::Double(_)
        | Value::Boolean(_)
        | Value::VerbatimString { .. }
        | Value::BigNumber(_)
        | Value::Push { .. }
        | Value::ServerError(_)
        | _ => Err(LockCacheError::UnexpectedReply {
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
