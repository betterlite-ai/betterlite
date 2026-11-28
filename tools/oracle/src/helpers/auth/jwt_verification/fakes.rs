use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::jwt_keys::cache::KeyCache;
use betterlite_auth::jwt_keys::clock::Clock;
use betterlite_auth::jwt_keys::errors::KeyError;
use betterlite_auth::jwt_keys::http::FetchError;
use betterlite_auth::jwt_verification::source::PublicKeySource;
use betterlite_auth::jwt_verification::userinfo::{UserinfoHttp, UserinfoReply};
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;

use super::input::{Reply, ReplyKind};
use super::output::{Request, Write};

// Every fake is reached while a step runs, so a poisoned lock carries on with its value.
pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// What the step's fixture picked up since the marker, so each answer carries its own effects.
pub(super) fn tail<T>(mutex: &Mutex<Vec<T>>, from: usize) -> Vec<T> {
    let mut items = lock(mutex);
    if from >= items.len() {
        return Vec::new();
    }
    items.split_off(from)
}

// The cache writes the userinfo path makes; a read answers what the same scenario wrote before.
#[derive(Default)]
pub(super) struct Cache {
    pub(super) entries: Mutex<BTreeMap<String, Json>>,
    pub(super) writes: Mutex<Vec<Write>>,
}

#[async_trait]
impl KeyCache for Cache {
    async fn read_value(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
        Ok(lock(&self.entries).get(key).cloned())
    }

    async fn write_value(
        &self,
        key: &str,
        value: &Json,
        ttl_seconds: f64,
    ) -> Result<(), AuthCacheError> {
        lock(&self.entries).insert(key.to_owned(), value.clone());
        lock(&self.writes).push(Write {
            key: key.to_owned(),
            ttl: ttl_seconds,
        });
        Ok(())
    }
}

// The wall clock a scenario holds still at, so exp and nbf never depend on the machine.
pub(super) struct FixedClock {
    pub(super) now: f64,
}
#[async_trait]
impl Clock for FixedClock {
    fn now(&self) -> f64 {
        self.now
    }
    async fn sleep(&self, _delay: Duration) {}
}
// The identity provider's one published key: the same answer for the global and the issuer lookup.
pub(super) struct Source {
    pub(super) key: Json,
}
#[async_trait]
impl PublicKeySource for Source {
    async fn resolve_public_key(&self, _kid: Option<&str>) -> Result<Json, KeyError> {
        Ok(self.key.clone())
    }
    async fn resolve_keys_for_url(
        &self,
        _jwks_url: &str,
        _kid: Option<&str>,
    ) -> Result<Json, KeyError> {
        Ok(self.key.clone())
    }
}
// One GET per userinfo call, answered from the step's script and never retried.
#[derive(Default)]
pub(super) struct ScriptedUserinfo {
    pub(super) replies: Mutex<VecDeque<Reply>>,
    pub(super) requests: Mutex<Vec<Request>>,
}
#[async_trait]
impl UserinfoHttp for ScriptedUserinfo {
    async fn get(
        &self,
        url: &str,
        authorization: &str,
        accept: &str,
    ) -> Result<UserinfoReply, FetchError> {
        lock(&self.requests).push(Request {
            url: url.to_owned(),
            authorization: authorization.to_owned(),
            accept: accept.to_owned(),
        });
        let answer = lock(&self.replies).pop_front();
        match answer {
            Some(Reply {
                kind: ReplyKind::Answer,
                status,
                body,
            }) => Ok(UserinfoReply {
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
    fn close(&self) {}
}
