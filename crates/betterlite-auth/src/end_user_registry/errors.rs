use crate::auth_cache::AuthCacheError;

// only a cache read failure reaches the caller: an unloadable registry degrades to per-id lookups instead
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("the registry could not be read from the cache")]
    CacheRead(#[source] AuthCacheError),
}
