use async_trait::async_trait;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_models::decisions::Json;

use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::keys::NO_TEAM_MEMBERSHIP;
use crate::auth_cache::payload::{self, PayloadError};
use crate::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};

// auth_checks.py's three outcomes; its _TeamMembershipCacheMiss marker is this Miss variant, never stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CachedMembership {
    Membership(Box<LiteLLMTeamMembership>),
    Absent,
    Miss,
}

// The memory tier can answer with a live model instance, so the classifier takes both shapes.
#[derive(Clone, Debug)]
pub enum CachedPayload {
    Pending,
    Typed(Box<LiteLLMTeamMembership>),
    Stored(Box<Json>),
}

impl From<Option<Json>> for CachedPayload {
    fn from(cached: Option<Json>) -> Self {
        match cached {
            Some(stored) => Self::Stored(Box::new(stored)),
            None => Self::Pending,
        }
    }
}

// None is a miss, the exact sentinel is a known absence, and every other payload goes through B4291's codec.
pub fn classify_cached(cached: CachedPayload) -> Result<CachedMembership, PayloadError> {
    let stored = match cached {
        CachedPayload::Typed(membership) => return Ok(CachedMembership::Membership(membership)),
        CachedPayload::Pending => return Ok(CachedMembership::Miss),
        CachedPayload::Stored(stored) => *stored,
    };
    let absent = matches!(&stored, Json::Text(text) if text.as_str() == NO_TEAM_MEMBERSHIP);
    if absent {
        return Ok(CachedMembership::Absent);
    }
    match payload::decode::<LiteLLMTeamMembership>(&stored)? {
        Some(membership) => Ok(CachedMembership::Membership(Box::new(membership))),
        None => Ok(CachedMembership::Miss),
    }
}

// The cache operations the lookup needs; the first read is local, and the load's read also reaches Redis.
#[async_trait]
pub trait MembershipCache: Send + Sync {
    fn management_ttl(&self) -> f64;
    async fn read(&self, key: &str, local_only: bool) -> Result<Option<Json>, AuthCacheError>;
    async fn write_membership(
        &self,
        key: &str,
        membership: &LiteLLMTeamMembership,
        ttl: Option<f64>,
    ) -> Result<(), AuthCacheError>;
    async fn write_absent(&self, key: &str, ttl: Option<f64>) -> Result<(), AuthCacheError>;
}

fn write_options(ttl: Option<f64>) -> WriteOptions {
    WriteOptions {
        ttl: ttl.map(|seconds| WriteTtl { seconds }),
        local_only: false,
        refresh_ttl: false,
    }
}

#[async_trait]
impl<T> MembershipCache for AuthCache<T>
where
    T: AuthCacheTier + 'static,
{
    fn management_ttl(&self) -> f64 {
        self.management_object_ttl()
    }

    // B733 logs a backend read failure and answers a miss, so this read has no failure of its own.
    async fn read(&self, key: &str, local_only: bool) -> Result<Option<Json>, AuthCacheError> {
        Ok(self.async_get(key, local_only).await)
    }

    async fn write_membership(
        &self,
        key: &str,
        membership: &LiteLLMTeamMembership,
        ttl: Option<f64>,
    ) -> Result<(), AuthCacheError> {
        self.async_set(Some(key), membership, write_options(ttl))
            .await
    }

    async fn write_absent(&self, key: &str, ttl: Option<f64>) -> Result<(), AuthCacheError> {
        self.async_set(Some(key), &NO_TEAM_MEMBERSHIP, write_options(ttl))
            .await
    }
}
