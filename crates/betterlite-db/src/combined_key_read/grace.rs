use chrono::{DateTime, NaiveDateTime, TimeDelta, Utc};
use indexmap::IndexMap;

use crate::key_hash::KeyHash;

pub const DEPRECATED_CACHE_TTL_SECONDS: i64 = 60;
pub const DEPRECATED_CACHE_CAPACITY: usize = 1000;

#[derive(Debug, Clone)]
pub struct DeprecatedToken {
    pub active_token_id: String,
    pub revoke_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum DeprecatedRowError {
    #[error("the deprecated token row has no active_token_id")]
    MissingActiveTokenId,
    #[error("the deprecated token row has no revoke_at")]
    MissingRevokeAt,
}

impl DeprecatedToken {
    pub fn from_columns(
        active_token_id: Option<String>,
        revoke_at: Option<NaiveDateTime>,
    ) -> Result<Self, DeprecatedRowError> {
        let active_token_id = active_token_id.ok_or(DeprecatedRowError::MissingActiveTokenId)?;
        let revoke_at = revoke_at.ok_or(DeprecatedRowError::MissingRevokeAt)?;
        Ok(Self {
            active_token_id,
            revoke_at: revoke_at.and_utc(),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CachedDeprecatedKey {
    active_token_id: String,
    expires_at: DateTime<Utc>,
    revoke_at: DateTime<Utc>,
}

// LiteLLM keeps only positive grace lookups, and treats a cached entry as expired at the boundary
#[derive(Debug, Default)]
pub struct DeprecatedKeyCache {
    pub(crate) entries: IndexMap<KeyHash, CachedDeprecatedKey>,
}

impl DeprecatedKeyCache {
    pub fn lookup(&mut self, hash: &KeyHash, now: DateTime<Utc>) -> Option<String> {
        let entry = self.entries.get(hash)?;
        if now < entry.expires_at && now < entry.revoke_at {
            return Some(entry.active_token_id.clone());
        }
        self.entries.shift_remove(hash);
        None
    }

    pub fn remember(&mut self, hash: KeyHash, deprecated: &DeprecatedToken, now: DateTime<Utc>) {
        if self.entries.len() >= DEPRECATED_CACHE_CAPACITY {
            drop(self.entries.shift_remove_index(0));
        }
        let ttl = TimeDelta::try_seconds(DEPRECATED_CACHE_TTL_SECONDS).unwrap_or_default();
        let expires_at = now
            .checked_add_signed(ttl)
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        self.entries.insert(
            hash,
            CachedDeprecatedKey {
                active_token_id: deprecated.active_token_id.clone(),
                expires_at,
                revoke_at: deprecated.revoke_at,
            },
        );
    }
}
