use serde::Serialize;
use serde_with::{DisplayFromStr, serde_as};
use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheSectionError {
    #[error("cache section unavailable: {source}")]
    Unavailable {
        #[source]
        source: io::Error,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CacheComponent {
    pub item_count: usize,
    pub cache_dict_bytes: u64,
    pub ttl_dict_bytes: u64,
}

#[serde_as]
#[derive(Debug, Serialize)]
pub struct CacheMemoryInput {
    #[serde_as(as = "Result<_, DisplayFromStr>")]
    pub user_api_key_cache: Result<CacheComponent, CacheSectionError>,
    #[serde_as(as = "Result<_, DisplayFromStr>")]
    pub llm_router_cache: Result<CacheComponent, CacheSectionError>,
    #[serde_as(as = "Result<_, DisplayFromStr>")]
    pub proxy_logging_cache: Result<CacheComponent, CacheSectionError>,
    #[serde_as(as = "Result<_, DisplayFromStr>")]
    pub redis_usage_cache: Result<CacheComponent, CacheSectionError>,
}

impl Default for CacheMemoryInput {
    fn default() -> Self {
        Self {
            user_api_key_cache: Ok(CacheComponent::default()),
            llm_router_cache: Ok(CacheComponent::default()),
            proxy_logging_cache: Ok(CacheComponent::default()),
            redis_usage_cache: Ok(CacheComponent::default()),
        }
    }
}

#[serde_as]
#[derive(Debug, Serialize)]
pub struct CacheMemorySnapshot {
    #[serde_as(as = "Result<_, DisplayFromStr>")]
    pub user_api_key_cache: Result<CacheComponent, CacheSectionError>,
    #[serde_as(as = "Result<_, DisplayFromStr>")]
    pub llm_router_cache: Result<CacheComponent, CacheSectionError>,
    #[serde_as(as = "Result<_, DisplayFromStr>")]
    pub proxy_logging_cache: Result<CacheComponent, CacheSectionError>,
    #[serde_as(as = "Result<_, DisplayFromStr>")]
    pub redis_usage_cache: Result<CacheComponent, CacheSectionError>,
}

impl CacheMemorySnapshot {
    #[must_use]
    pub fn from_input(input: CacheMemoryInput) -> Self {
        let user_api_key_cache = input.user_api_key_cache;
        let llm_router_cache = input.llm_router_cache;
        let proxy_logging_cache = input.proxy_logging_cache;
        let redis_usage_cache = input.redis_usage_cache;
        Self {
            user_api_key_cache,
            llm_router_cache,
            proxy_logging_cache,
            redis_usage_cache,
        }
    }
}
