#![cfg(test)]

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use thiserror::Error;

use super::{AsyncCache, DualCacheTokenCacheBackend};
use crate::mcp::outbound_credentials::cache_codec::OAuthTokenCacheCodec;
use crate::mcp::outbound_credentials::{
    OAuthToken, PER_USER_TOKEN_KEY_PREFIX, TokenCacheBackend as _, TokenKey,
};

#[derive(Debug, Error)]
#[error("the cache is unavailable")]
struct CacheUnavailable;

enum GetBehavior {
    Returns(Option<CacheValue>),
    Fails,
}

struct FakeCache {
    get_behavior: GetBehavior,
    set_fails: bool,
    delete_fails: bool,
    gets: Mutex<Vec<String>>,
    sets: Mutex<Vec<(String, String, Option<Duration>)>>,
    deletes: Mutex<Vec<String>>,
}

impl FakeCache {
    fn returning(value: Option<CacheValue>) -> Self {
        Self {
            get_behavior: GetBehavior::Returns(value),
            set_fails: false,
            delete_fails: false,
            gets: Mutex::new(Vec::new()),
            sets: Mutex::new(Vec::new()),
            deletes: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl AsyncCache for FakeCache {
    type Error = CacheUnavailable;

    async fn async_get_cache(&self, key: &str) -> Result<Option<CacheValue>, CacheUnavailable> {
        self.gets.lock().unwrap().push(key.to_owned());
        match &self.get_behavior {
            GetBehavior::Returns(value) => Ok(value.clone()),
            GetBehavior::Fails => Err(CacheUnavailable),
        }
    }

    async fn async_set_cache(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), CacheUnavailable> {
        if self.set_fails {
            return Err(CacheUnavailable);
        }
        self.sets
            .lock()
            .unwrap()
            .push((key.to_owned(), value.to_owned(), ttl));
        Ok(())
    }

    async fn async_delete_cache(&self, key: &str) -> Result<(), CacheUnavailable> {
        if self.delete_fails {
            return Err(CacheUnavailable);
        }
        self.deletes.lock().unwrap().push(key.to_owned());
        Ok(())
    }
}

fn codec() -> OAuthTokenCacheCodec {
    OAuthTokenCacheCodec {
        encrypt: Box::new(|value: &str| format!("enc:{value}")),
        decrypt: Box::new(|blob: &str| blob.strip_prefix("enc:").map(str::to_owned)),
    }
}

fn backend(cache: FakeCache) -> DualCacheTokenCacheBackend<FakeCache> {
    DualCacheTokenCacheBackend {
        cache,
        codec: codec(),
        key_prefix: PER_USER_TOKEN_KEY_PREFIX,
    }
}

fn token(access_token: &str) -> OAuthToken {
    OAuthToken {
        access_token: access_token.to_owned(),
        expires_at: None,
        refresh_token: None,
        scopes: Vec::new(),
        identity_binding_proof: None,
    }
}

fn key() -> TokenKey {
    TokenKey {
        user_id: "user".to_owned(),
        server_id: "server".to_owned(),
    }
}

#[tokio::test]
async fn dual_cache_get_decodes_a_text_value() {
    let cache = FakeCache::returning(Some(CacheValue::Text("enc:access".to_owned())));
    let backend = backend(cache);
    assert_eq!(backend.get(&key()).await, Some(token("access")));
}

#[tokio::test]
async fn dual_cache_get_returns_none_for_a_non_text_value() {
    let cache = FakeCache::returning(Some(CacheValue::Integer(7)));
    let backend = backend(cache);
    assert_eq!(backend.get(&key()).await, None);
}

#[tokio::test]
async fn dual_cache_get_returns_none_for_a_missing_value() {
    let cache = FakeCache::returning(None);
    let backend = backend(cache);
    assert_eq!(backend.get(&key()).await, None);
}

#[tokio::test]
async fn dual_cache_get_returns_none_when_the_codec_cannot_decrypt() {
    let cache = FakeCache::returning(Some(CacheValue::Text("not-encrypted".to_owned())));
    let backend = backend(cache);
    assert_eq!(backend.get(&key()).await, None);
}

#[tokio::test]
async fn dual_cache_get_degrades_a_cache_failure_to_a_miss() {
    let cache = FakeCache {
        get_behavior: GetBehavior::Fails,
        set_fails: false,
        delete_fails: false,
        gets: Mutex::new(Vec::new()),
        sets: Mutex::new(Vec::new()),
        deletes: Mutex::new(Vec::new()),
    };
    let backend = backend(cache);
    assert_eq!(backend.get(&key()).await, None);
}

#[tokio::test]
async fn dual_cache_set_encodes_the_token_and_passes_the_ttl() {
    let cache = FakeCache::returning(None);
    let backend = backend(cache);
    backend
        .set(&key(), &token("access"), Duration::from_secs(30))
        .await;
    assert_eq!(
        *backend.cache.sets.lock().unwrap(),
        vec![(
            "mcp:per_user_token:user:server".to_owned(),
            "enc:access".to_owned(),
            Some(Duration::from_secs(30))
        )]
    );
}

#[tokio::test]
async fn dual_cache_set_failure_is_ignored() {
    let cache = FakeCache {
        get_behavior: GetBehavior::Returns(None),
        set_fails: true,
        delete_fails: false,
        gets: Mutex::new(Vec::new()),
        sets: Mutex::new(Vec::new()),
        deletes: Mutex::new(Vec::new()),
    };
    let backend = backend(cache);
    backend
        .set(&key(), &token("access"), Duration::from_secs(30))
        .await;
    assert!(backend.cache.sets.lock().unwrap().is_empty());
}

#[tokio::test]
async fn dual_cache_set_with_a_ttl_of_zero_writes_nothing() {
    let cache = FakeCache::returning(None);
    let backend = backend(cache);
    backend.set(&key(), &token("access"), Duration::ZERO).await;
    assert!(backend.cache.sets.lock().unwrap().is_empty());
}

#[tokio::test]
async fn dual_cache_delete_removes_the_v1_key() {
    let cache = FakeCache::returning(None);
    let backend = backend(cache);
    backend.delete(&key()).await;
    assert_eq!(
        *backend.cache.deletes.lock().unwrap(),
        vec!["mcp:per_user_token:user:server".to_owned()]
    );
}

#[tokio::test]
async fn dual_cache_delete_failure_is_a_noop() {
    let cache = FakeCache {
        get_behavior: GetBehavior::Returns(None),
        set_fails: false,
        delete_fails: true,
        gets: Mutex::new(Vec::new()),
        sets: Mutex::new(Vec::new()),
        deletes: Mutex::new(Vec::new()),
    };
    let backend = backend(cache);
    backend.delete(&key()).await;
    assert!(backend.cache.deletes.lock().unwrap().is_empty());
}
