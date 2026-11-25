mod tests;

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Mutex, PoisonError};

use chrono::Utc;
use thiserror::Error;
use tracing::warn;

use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode};

pub const CLAIM_UNAVAILABLE_DESCRIPTION: &str =
    "the single-use record is unavailable right now; try again shortly";

// At capacity only expired claims go, so a live claim is never evicted and cannot be replayed.
const MAX_MEMORY_CLAIMS: usize = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaimOutcome {
    First,
    Replayed,
    Unavailable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeekOutcome {
    Unclaimed,
    Claimed,
    Unavailable,
}
#[derive(Debug, Error)]
#[error("the single-use claim backend is unavailable")]
pub struct ClaimBackendError;

pub trait ClaimBackend: Send + Sync {
    fn increment(
        &self,
        key: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<u64, ClaimBackendError>> + Send;

    fn read(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<u64>, ClaimBackendError>> + Send;
}

pub trait ClaimClock: Send + Sync {
    fn now_seconds(&self) -> i64;
}

#[derive(Debug, Default)]
pub struct SystemClaimClock;

impl ClaimClock for SystemClaimClock {
    fn now_seconds(&self) -> i64 {
        Utc::now().timestamp()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ClaimRecord {
    count: u64,
    expires_at: i64,
}

pub struct MemoryClaimStore {
    pub clock: Box<dyn ClaimClock>,
    pub entries: Mutex<HashMap<String, ClaimRecord>>,
}

impl MemoryClaimStore {
    #[must_use]
    pub fn increment(&self, key: &str, ttl_seconds: i64) -> u64 {
        let now = self.clock.now_seconds();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries.len() >= MAX_MEMORY_CLAIMS {
            entries.retain(|_key, record| record.expires_at > now);
        }
        let expires_at = now.saturating_add(ttl_seconds);
        let record = entries.entry(key.to_owned()).or_insert(ClaimRecord {
            count: 0,
            expires_at,
        });
        if record.expires_at <= now {
            record.count = 0;
            record.expires_at = expires_at;
        }
        record.count = record.count.saturating_add(1);
        let count = record.count;
        drop(entries);
        count
    }

    pub fn read(&self, key: &str) -> Option<u64> {
        let now = self.clock.now_seconds();
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        match entries.get(key) {
            Some(record) if record.expires_at > now => Some(record.count),
            None | Some(_) => None,
        }
    }
}

pub enum ClaimAuthority<S> {
    Shared(S),
    Local(MemoryClaimStore),
}

pub struct SingleUseGuard<S> {
    authority: ClaimAuthority<S>,
}

impl<S: ClaimBackend> From<ClaimAuthority<S>> for SingleUseGuard<S> {
    fn from(authority: ClaimAuthority<S>) -> Self {
        Self { authority }
    }
}

impl<S: ClaimBackend> SingleUseGuard<S> {
    #[must_use]
    pub async fn claim(&self, key: &str, ttl_seconds: i64) -> ClaimOutcome {
        let counted = match &self.authority {
            ClaimAuthority::Shared(backend) => backend.increment(key, ttl_seconds).await,
            ClaimAuthority::Local(store) => Ok(store.increment(key, ttl_seconds)),
        };
        claimed(counted)
    }

    #[must_use]
    pub async fn peek(&self, key: &str) -> PeekOutcome {
        let count = match &self.authority {
            ClaimAuthority::Shared(backend) => backend.read(key).await,
            ClaimAuthority::Local(store) => Ok(store.read(key)),
        };
        match count {
            Ok(Some(_count)) => PeekOutcome::Claimed,
            Ok(None) => PeekOutcome::Unclaimed,
            Err(error) => {
                warn!(
                    error = %error,
                    "mcp gateway single-use peek: shared cache backend unavailable"
                );
                PeekOutcome::Unavailable
            }
        }
    }
}

fn claimed(counted: Result<u64, ClaimBackendError>) -> ClaimOutcome {
    match counted {
        Ok(1) => ClaimOutcome::First,
        Ok(_count) => ClaimOutcome::Replayed,
        Err(error) => {
            warn!(
                error = %error,
                "mcp gateway single-use claim: shared cache backend unavailable, failing closed"
            );
            ClaimOutcome::Unavailable
        }
    }
}

#[must_use]
pub fn claim_refusal(outcome: ClaimOutcome, replayed_error: OauthError) -> Option<OauthError> {
    match outcome {
        ClaimOutcome::First => None,
        ClaimOutcome::Replayed => Some(replayed_error),
        ClaimOutcome::Unavailable => Some(OauthError {
            status: 503,
            error: OauthErrorCode::TemporarilyUnavailable,
            description: CLAIM_UNAVAILABLE_DESCRIPTION.to_owned(),
        }),
    }
}
