use async_trait::async_trait;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::payload::PayloadError;
use crate::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};
use crate::end_user_registry::cache::RegistryCache;

// the two budget rows LiteLLM caches by id; the end-user row reuses B733's end_user_id key
pub const DEFAULT_END_USER_BUDGET_PREFIX: &str = "default_end_user_budget:";
pub const TEAM_MEMBER_BUDGET_PREFIX: &str = "team_member_default_budget:";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndUserCacheKey<'key> {
    DefaultEndUserBudget(&'key str),
    TeamMemberDefaultBudget(&'key str),
}

impl EndUserCacheKey<'_> {
    #[must_use]
    pub fn render(self) -> String {
        match self {
            Self::DefaultEndUserBudget(id) => format!("{DEFAULT_END_USER_BUDGET_PREFIX}{id}"),
            Self::TeamMemberDefaultBudget(id) => {
                format!("{TEAM_MEMBER_BUDGET_PREFIX}{id}")
            }
        }
    }
}

// The cache surface the end-user lookup reads: B1218's registry plus the typed rows this module caches.
#[async_trait]
pub trait EndUserCache: RegistryCache {
    async fn read_row<Row>(&self, cache_key: &str) -> Result<Option<Row>, PayloadError>
    where
        Row: DeserializeOwned + Send + 'static;

    async fn write_row<Row>(
        &self,
        cache_key: &str,
        row: &Row,
        ttl: f64,
    ) -> Result<(), AuthCacheError>
    where
        Row: Serialize + Sync;
}

#[async_trait]
impl<T: AuthCacheTier + 'static> EndUserCache for AuthCache<T> {
    async fn read_row<Row>(&self, cache_key: &str) -> Result<Option<Row>, PayloadError>
    where
        Row: DeserializeOwned + Send + 'static,
    {
        self.async_get_typed(cache_key, false).await
    }

    async fn write_row<Row>(
        &self,
        cache_key: &str,
        row: &Row,
        ttl: f64,
    ) -> Result<(), AuthCacheError>
    where
        Row: Serialize + Sync,
    {
        let options = WriteOptions {
            ttl: Some(WriteTtl { seconds: ttl }),
            local_only: false,
            refresh_ttl: false,
        };
        self.async_set(Some(cache_key), row, options).await
    }
}
