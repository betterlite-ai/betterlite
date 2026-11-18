use std::future::Future;
use std::sync::Arc;

use betterlite_redis::cache_primitives::decode::{CachedValue, validate_lua_count};
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_counters::scripts::RegisteredScript;
use num_traits::ToPrimitive as _;
use redis::Value;

use super::owner::COMPARE_AND_DELETE_LOCK_SCRIPT;
use super::{LockCache, LockCacheError, LockScript};

mod tests;

// B126's RedisCache as the lease port sees it: async_set_cache with NX and EX stores the JSON string,
// async_get_cache hands the owner back as text, and async_register_script caches the compare-and-delete Lua.
pub struct RedisLockCache {
    pub cache: Arc<RedisCache>,
}

impl LockCache for RedisLockCache {
    #[rustfmt::skip]
    fn get_owner(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>, LockCacheError>> + Send {
        async move {
            let value = self
                .cache
                .async_get_cache(key)
                .await
                .map_err(|source| LockCacheError::Cache {
                    operation: "async_get_cache",
                    source,
                })?;
            Ok(value.and_then(owner_bytes))
        }
    }

    // A live cache needs a running Redis: db tests fake at the LockCache seam, and the oracle fixture tests run this adapter against real Redis.
    #[cfg_attr(test, mutants::skip)]
    #[rustfmt::skip]
    fn set_owner_if_absent(&self, key: &str, owner: &str, ttl_seconds: i64) -> impl Future<Output = Result<bool, LockCacheError>> + Send {
        async move {
            let ttl = ttl_seconds.to_f64().unwrap_or_default();
            let stored = self
                .cache
                .values
                .async_set_cache(Some(key), owner, Some(ttl), true)
                .await
                .map_err(|source| LockCacheError::Values {
                    operation: "async_set_cache",
                    source,
                })?;
            Ok(stored == Some(true))
        }
    }

    #[rustfmt::skip]
    fn delete(&self, key: &str) -> impl Future<Output = Result<i64, LockCacheError>> + Send {
        async move {
            self.cache
                .admin
                .async_delete_cache(key)
                .await
                .map_err(|source| LockCacheError::Admin {
                    operation: "async_delete_cache",
                    source,
                })
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

struct RedisLockScript {
    cache: Arc<RedisCache>,
    script: RegisteredScript,
}

impl LockScript for RedisLockScript {
    #[rustfmt::skip]
    fn run(&self, key: &str, owner: &str) -> impl Future<Output = Result<bool, LockCacheError>> + Send {
        async move {
            // async_set_cache stored json.dumps(pod_id), so the script's ARGV[1] must carry the same quoted text.
            let owner_json = serde_json::to_string(owner)?;
            let reply = self
                .cache
                .run_script(&self.script, &[key.to_owned()], &[RedisScalar::Text(owner_json)])
                .await
                .map_err(|source| LockCacheError::Cache {
                    operation: "run_script",
                    source,
                })?;
            deleted_one(reply)
        }
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
