use std::collections::{BTreeMap, VecDeque};
use std::ops::Add as _;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::jwt_keys::cache::KeyCache;
use betterlite_auth::jwt_keys::clock::Clock;
use betterlite_auth::jwt_keys::http::{FetchError, HttpReply, JwksHttp};
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;

use super::input::{Reply, ReplyKind};
use super::output::Write;

// Every fake is reached from the resolver while a step runs, so a poisoned lock carries on with its value.
pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// The wall clock a step reads: it holds still inside a step and jumps to the step's own value between them.
#[derive(Default)]
pub(super) struct StepClock {
    pub(super) now: Mutex<f64>,
    pub(super) sleeps: Mutex<Vec<f64>>,
}

#[async_trait]
impl Clock for StepClock {
    fn now(&self) -> f64 {
        *lock(&self.now)
    }

    async fn sleep(&self, delay: Duration) {
        lock(&self.sleeps).push(delay.as_secs_f64());
    }
}

pub(super) struct Entry {
    value: Json,
    expires_at: f64,
}

// InMemoryCache keeps a written key readable until the clock passes its expiry, and a re-write keeps an unexpired one.
pub(super) struct Cache {
    pub(super) entries: Mutex<BTreeMap<String, Entry>>,
    pub(super) writes: Mutex<Vec<Write>>,
    pub(super) clock: Arc<StepClock>,
}

#[async_trait]
impl KeyCache for Cache {
    async fn read_value(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
        let now = *lock(&self.clock.now);
        let mut entries = lock(&self.entries);
        let Some(entry) = entries.get(key) else {
            return Ok(None);
        };
        if now > entry.expires_at {
            entries.remove(key);
            return Ok(None);
        }
        let value = entry.value.clone();
        drop(entries);
        Ok(Some(value))
    }

    async fn write_value(
        &self,
        key: &str,
        value: &Json,
        ttl_seconds: f64,
    ) -> Result<(), AuthCacheError> {
        let now = *lock(&self.clock.now);
        let mut entries = lock(&self.entries);
        let expires_at = entries
            .get(key)
            .map(|entry| entry.expires_at)
            .filter(|expires| now <= *expires)
            .unwrap_or_else(|| now.add(ttl_seconds));
        entries.insert(
            key.to_owned(),
            Entry {
                value: value.clone(),
                expires_at,
            },
        );
        drop(entries);
        lock(&self.writes).push(Write {
            key: key.to_owned(),
            ttl: ttl_seconds,
        });
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Scripted {
    pub(super) replies: Mutex<VecDeque<Reply>>,
    pub(super) requests: Mutex<Vec<String>>,
}

// A step whose script runs out answers like an unreachable provider, so a mismatch fails on its counted requests.
#[async_trait]
impl JwksHttp for Scripted {
    async fn get(&self, url: &str) -> Result<HttpReply, FetchError> {
        lock(&self.requests).push(url.to_owned());
        let reply = lock(&self.replies).pop_front();
        match reply {
            Some(Reply {
                kind: ReplyKind::Answer,
                status,
                body,
            }) => Ok(HttpReply {
                status,
                body: body.into_bytes(),
            }),
            Some(Reply {
                kind: ReplyKind::Transport,
                ..
            })
            | None => Err(FetchError::Transport(HttpError::Timeout)),
        }
    }
}
