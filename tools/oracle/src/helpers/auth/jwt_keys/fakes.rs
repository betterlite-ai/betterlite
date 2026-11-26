use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::jwt_keys::cache::KeyCache;
use betterlite_auth::jwt_keys::clock::Clock;
use betterlite_auth::jwt_keys::http::{FetchError, HttpReply, JwksHttp};
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;
use serde_json::Value;

use super::input::{Reply, ReplyKind};

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
    now: Mutex<f64>,
    sleeps: Mutex<Vec<f64>>,
}

impl StepClock {
    pub(super) fn set(&self, now: f64) {
        *lock(&self.now) = now;
    }

    pub(super) fn current(&self) -> f64 {
        *lock(&self.now)
    }

    pub(super) fn sleeps_len(&self) -> usize {
        lock(&self.sleeps).len()
    }

    pub(super) fn sleeps_since(&self, index: usize) -> Vec<f64> {
        lock(&self.sleeps).iter().skip(index).copied().collect()
    }
}

#[async_trait]
impl Clock for StepClock {
    fn now(&self) -> f64 {
        self.current()
    }

    async fn sleep(&self, delay: Duration) {
        lock(&self.sleeps).push(delay.as_secs_f64());
    }
}

struct Entry {
    value: Json,
    expires_at: f64,
}

// LiteLLM's InMemoryCache keeps a written key readable until the clock passes its expiry, and a second write
// while it is unexpired replaces the value but not the expiry.
pub(super) struct Cache {
    entries: Mutex<BTreeMap<String, Entry>>,
    writes: Mutex<Vec<Value>>,
    clock: Arc<StepClock>,
}

impl Cache {
    pub(super) fn new(clock: Arc<StepClock>) -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            writes: Mutex::new(Vec::new()),
            clock,
        }
    }

    pub(super) fn writes_len(&self) -> usize {
        lock(&self.writes).len()
    }

    pub(super) fn writes_since(&self, index: usize) -> Vec<Value> {
        lock(&self.writes).iter().skip(index).cloned().collect()
    }
}

#[async_trait]
impl KeyCache for Cache {
    async fn get(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
        let now = self.clock.current();
        let mut entries = lock(&self.entries);
        let Some(entry) = entries.get(key) else {
            return Ok(None);
        };
        if now > entry.expires_at {
            entries.remove(key);
            return Ok(None);
        }
        Ok(Some(entry.value.clone()))
    }

    async fn set(&self, key: &str, value: &Json, ttl_seconds: f64) -> Result<(), AuthCacheError> {
        let now = self.clock.current();
        {
            let mut entries = lock(&self.entries);
            let expires_at = entries
                .get(key)
                .map(|entry| entry.expires_at)
                .filter(|expires| now <= *expires)
                .unwrap_or(now + ttl_seconds);
            entries.insert(
                key.to_owned(),
                Entry {
                    value: value.clone(),
                    expires_at,
                },
            );
        }
        lock(&self.writes).push(serde_json::json!({"key": key, "ttl": ttl_seconds}));
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Scripted {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<String>>,
}

impl Scripted {
    pub(super) fn script(&self, replies: &[Reply]) {
        lock(&self.replies).extend(replies.iter().cloned());
    }

    pub(super) fn requests_len(&self) -> usize {
        lock(&self.requests).len()
    }

    pub(super) fn requests_since(&self, index: usize) -> Vec<String> {
        lock(&self.requests).iter().skip(index).cloned().collect()
    }
}

// A step whose script runs out answers like an unreachable provider, so the case fails on its counted requests
// rather than on a panic.
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
