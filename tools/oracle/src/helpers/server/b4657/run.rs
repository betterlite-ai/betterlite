use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_mcp::credential_contracts::errors::CredError;
use betterlite_server::mcp::outbound_credentials::distributed_lock::{
    DistributedLock, LockAcquisition,
};
use betterlite_server::mcp::outbound_credentials::refresh_coordinator::{
    DEFAULT_KEY_PREFIX, DEFAULT_LOCK_TTL, DEFAULT_REFRESH_BUDGET, DEFAULT_WAIT_TIMEOUT,
    RedisRefreshCoordinator, RedisTunables,
};
use betterlite_server::mcp::outbound_credentials::token_endpoint::ExchangedToken;
use betterlite_server::mcp::outbound_credentials::token_endpoint::cache::ExchangedTokenCache;
use betterlite_server::mcp::outbound_credentials::token_exchange_provider::TokenExchangeTunables;
use betterlite_server::mcp::outbound_credentials::{
    RefreshCoordinator as _, RefreshFuture, RefreshStep, SystemWallClock, TokenKey,
};
use serde::Deserialize;
use serde_json::Value;

use super::support::{cred_tag, is_hex_32, runtime, value_error};
use crate::helpers::HelperResult;

const POLL_INTERVAL: Duration = Duration::from_millis(5);
const REFRESH_DELAY: Duration = Duration::from_millis(50);

type Coordinator = RedisRefreshCoordinator<FakeLock, String, String>;

#[derive(Deserialize)]
struct CacheInput {
    #[serde(default)]
    operations: Vec<Operation>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Operation {
    GetOrCompute {
        key: String,
        #[serde(default)]
        fingerprint: String,
        compute: Compute,
    },
    Invalidate {
        key: String,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Compute {
    Token {
        access_token: String,
        expires_in: Option<i64>,
    },
    Error {
        detail: String,
    },
}

impl Compute {
    fn future<'call>(
        &'call self,
        calls: &'call AtomicUsize,
    ) -> RefreshFuture<'call, ExchangedToken, CredError> {
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            match self {
                Compute::Token {
                    access_token,
                    expires_in,
                } => Ok(ExchangedToken {
                    access_token: access_token.clone(),
                    expires_in: *expires_in,
                }),
                Compute::Error { detail } => Err(CredError::UpstreamUnavailable(detail.clone())),
            }
        })
    }
}

pub(super) fn cache(input: &Value) -> HelperResult {
    let parsed: CacheInput =
        serde_json::from_value(input.clone()).map_err(|error| value_error(error.to_string()))?;
    let cache =
        ExchangedTokenCache::new(Arc::new(SystemWallClock), TokenExchangeTunables::default());
    let calls = AtomicUsize::new(0);
    let results = runtime()?.block_on(async {
        let mut results: Vec<Value> = Vec::new();
        for operation in &parsed.operations {
            match operation {
                Operation::Invalidate { key } => cache.invalidate(key),
                Operation::GetOrCompute {
                    key,
                    fingerprint,
                    compute,
                } => {
                    let outcome = cache
                        .get_or_compute(key, fingerprint, compute.future(&calls))
                        .await;
                    results.push(match outcome {
                        Ok(token) => serde_json::json!({"ok": true, "token": token}),
                        Err(error) => {
                            serde_json::json!({"ok": false, "error": cred_tag(&error)})
                        }
                    });
                }
            }
        }
        results
    });
    Ok(serde_json::json!({
        "results": results,
        "compute_calls": calls.load(Ordering::SeqCst),
    }))
}

#[derive(Default)]
struct FakeLock {
    held: AtomicBool,
    acquisitions: Mutex<Vec<&'static str>>,
    releases: AtomicUsize,
    extends: AtomicUsize,
    key: Mutex<String>,
    token: Mutex<String>,
}

#[async_trait]
impl DistributedLock for FakeLock {
    async fn acquire(&self, key: &str, token: &str, _ttl: Duration) -> LockAcquisition {
        *self.key.lock().unwrap_or_else(PoisonError::into_inner) = key.to_owned();
        *self.token.lock().unwrap_or_else(PoisonError::into_inner) = token.to_owned();
        let mut acquisitions = self
            .acquisitions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.held.swap(true, Ordering::SeqCst) {
            acquisitions.push("held");
            LockAcquisition::Held
        } else {
            acquisitions.push("acquired");
            LockAcquisition::Acquired
        }
    }

    async fn extend(&self, _key: &str, _token: &str, _ttl: Duration) -> bool {
        self.extends.fetch_add(1, Ordering::SeqCst);
        true
    }

    async fn release(&self, _key: &str, _token: &str) {
        self.releases.fetch_add(1, Ordering::SeqCst);
        self.held.store(false, Ordering::SeqCst);
    }

    async fn is_held(&self, _key: &str) -> bool {
        self.held.load(Ordering::SeqCst)
    }
}

fn refresh_step<'call>(refreshes: &'call AtomicUsize) -> RefreshStep<'call, String, String> {
    Box::new(move || {
        refreshes.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            tokio::time::sleep(REFRESH_DELAY).await;
            Ok("fresh".to_owned())
        })
    })
}

fn reread_step<'call>(rereads: &'call AtomicUsize) -> RefreshStep<'call, String, String> {
    Box::new(move || {
        rereads.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok("reread".to_owned()) })
    })
}

fn token_key() -> TokenKey {
    TokenKey {
        user_id: "user".to_owned(),
        server_id: "server".to_owned(),
    }
}

async fn run_pair(
    coordinator: &Coordinator,
    refreshes: &AtomicUsize,
    rereads: &AtomicUsize,
) -> Value {
    let winner = coordinator.run(token_key(), refresh_step(refreshes), reread_step(rereads));
    let loser = coordinator.run(token_key(), refresh_step(refreshes), reread_step(rereads));
    let (winner, loser) = tokio::join!(winner, loser);
    let (Ok(winner), Ok(loser)) = (winner, loser) else {
        return serde_json::json!({"error": "coordinator returned an error"});
    };
    let acquisitions = coordinator
        .lock
        .acquisitions
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let key = coordinator
        .lock
        .key
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let token = coordinator
        .lock
        .token
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    serde_json::json!({
        "acquisitions": acquisitions,
        "refresh_calls": refreshes.load(Ordering::SeqCst),
        "reread_calls": rereads.load(Ordering::SeqCst),
        "release_calls": coordinator.lock.releases.load(Ordering::SeqCst),
        "extend_calls": coordinator.lock.extends.load(Ordering::SeqCst),
        "key": key,
        "token_is_hex_32": is_hex_32(&token),
        "winner": winner,
        "loser": loser,
    })
}

pub(super) fn coordinator(_input: &Value) -> HelperResult {
    let refreshes = AtomicUsize::new(0);
    let rereads = AtomicUsize::new(0);
    let mut coordinator = Coordinator::new(FakeLock::default());
    coordinator.tunables = RedisTunables {
        key_prefix: DEFAULT_KEY_PREFIX.to_owned(),
        lock_ttl: DEFAULT_LOCK_TTL,
        refresh_budget: DEFAULT_REFRESH_BUDGET,
        wait_timeout: DEFAULT_WAIT_TIMEOUT,
        poll_interval: POLL_INTERVAL,
    };
    let view = runtime()?.block_on(run_pair(&coordinator, &refreshes, &rereads));
    Ok(view)
}
